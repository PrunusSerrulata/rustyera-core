//! Portable, I/O-free representation of the Era project configuration catalog.
//!
//! This crate deliberately stores client-specific settings as compatibility values.
//! Consumers decide which settings have portable runtime semantics; parsing a setting
//! never grants access to a device or forces a frontend rendering choice.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

mod rera;

pub use rera::{
    ByteSpan, LegacyConfigSource, LegacyMigration, LegacyMigrationDiagnostic,
    LegacyMigrationDiagnosticKind, RERACONFIG_SCHEMA_VERSION, ReraConfigDocument, ReraConfigError,
    ReraConfigErrorKind, ReraConfigSpec, generate_annotated_example, generate_json_schema,
    migrate_legacy_configuration, normalize_line_endings, rera_catalog,
};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ConfigValue {
    Boolean(bool),
    Integer(i64),
    String(String),
    Enum { value: String, allowed: Vec<String> },
    Color(u32),
    Character(char),
    IntegerList(Vec<i64>),
    StringList(Vec<String>),
}

impl ConfigValue {
    /// Render the value using the canonical syntax accepted by Emuera config files.
    #[must_use]
    pub fn config_text(&self) -> String {
        match self {
            Self::Boolean(value) => if *value { "YES" } else { "NO" }.into(),
            Self::Integer(value) => value.to_string(),
            Self::String(value) | Self::Enum { value, .. } => value.clone(),
            Self::Color(value) => format!(
                "{},{},{}",
                (value >> 16) & 0xff,
                (value >> 8) & 0xff,
                value & 0xff
            ),
            Self::Character(value) => value.to_string(),
            Self::IntegerList(values) => values
                .iter()
                .map(i64::to_string)
                .collect::<Vec<_>>()
                .join("/"),
            Self::StringList(values) => values.join(","),
        }
    }

    /// Convert to the exact scalar family exposed by GETCONFIG/GETCONFIGS.
    #[must_use]
    pub fn script_value(&self) -> ScriptConfigValue {
        match self {
            Self::Boolean(value) => ScriptConfigValue::Integer(i64::from(*value)),
            Self::Integer(value) => ScriptConfigValue::Integer(*value),
            Self::Color(value) => ScriptConfigValue::Integer(i64::from(*value)),
            Self::String(value) | Self::Enum { value, .. } => {
                ScriptConfigValue::String(value.clone())
            }
            Self::Character(value) => ScriptConfigValue::String(value.to_string()),
            // The pinned reference falls through to List<Int64>.ToString() here.
            // Keep that odd script-visible result rather than exposing a nicer list.
            Self::IntegerList(_) => {
                ScriptConfigValue::String("System.Collections.Generic.List`1[System.Int64]".into())
            }
            Self::StringList(values) => ScriptConfigValue::String(values.join(",")),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ScriptConfigValue {
    Integer(i64),
    String(String),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigEffect {
    PortableSemantic,
    QueryOnlyClientPreference,
    UnsupportedPlatformIntegration,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigClient {
    Runtime,
    Tui,
    Browser,
    Tauri,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigApplication {
    Hot,
    Restart,
}

#[derive(Clone, Debug)]
pub struct ConfigSpec {
    pub code: &'static str,
    pub japanese: &'static str,
    pub english: &'static str,
    pub default: ConfigValue,
    pub effect: ConfigEffect,
    pub clients: &'static [ConfigClient],
}

macro_rules! spec {
    ($code:ident, $jp:literal, $en:literal, b $value:expr, $effect:ident) => {
        ConfigSpec { code: stringify!($code), japanese: $jp, english: $en, default: ConfigValue::Boolean($value), effect: ConfigEffect::$effect, clients: &[] }
    };
    ($code:ident, $jp:literal, $en:literal, i $value:expr, $effect:ident) => {
        ConfigSpec { code: stringify!($code), japanese: $jp, english: $en, default: ConfigValue::Integer($value), effect: ConfigEffect::$effect, clients: &[] }
    };
    ($code:ident, $jp:literal, $en:literal, s $value:literal, $effect:ident) => {
        ConfigSpec { code: stringify!($code), japanese: $jp, english: $en, default: ConfigValue::String($value.into()), effect: ConfigEffect::$effect, clients: &[] }
    };
    ($code:ident, $jp:literal, $en:literal, e $value:literal [$($allowed:literal),+ $(,)?], $effect:ident) => {
        ConfigSpec { code: stringify!($code), japanese: $jp, english: $en, default: ConfigValue::Enum { value: $value.into(), allowed: vec![$($allowed.into()),+] }, effect: ConfigEffect::$effect, clients: &[] }
    };
    ($code:ident, $jp:literal, $en:literal, c $value:expr, $effect:ident) => {
        ConfigSpec { code: stringify!($code), japanese: $jp, english: $en, default: ConfigValue::Color($value), effect: ConfigEffect::$effect, clients: &[] }
    };
    ($code:ident, $jp:literal, $en:literal, ch $value:literal, $effect:ident) => {
        ConfigSpec { code: stringify!($code), japanese: $jp, english: $en, default: ConfigValue::Character($value), effect: ConfigEffect::$effect, clients: &[] }
    };
    ($code:ident, $jp:literal, $en:literal, il [$($value:expr),* $(,)?], $effect:ident) => {
        ConfigSpec { code: stringify!($code), japanese: $jp, english: $en, default: ConfigValue::IntegerList(vec![$($value),*]), effect: ConfigEffect::$effect, clients: &[] }
    };
    ($code:ident, $jp:literal, $en:literal, sl [$($value:literal),* $(,)?], $effect:ident) => {
        ConfigSpec { code: stringify!($code), japanese: $jp, english: $en, default: ConfigValue::StringList(vec![$($value.into()),*]), effect: ConfigEffect::$effect, clients: &[] }
    };
}

/// Return the complete catalog constructed by the pinned `ConfigData.setDefault`.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn catalog() -> Vec<ConfigSpec> {
    use ConfigEffect::{
        PortableSemantic as P, QueryOnlyClientPreference as Q, UnsupportedPlatformIntegration as U,
    };
    let _ = (P, Q, U); // Keep the classification names visible in generated documentation.
    let mut specs = vec![
        spec!(IgnoreCase, "大文字小文字の違いを無視する", "Ignore case", b true, PortableSemantic),
        spec!(UseRenameFile, "_Rename.csvを利用する", "Use _Rename.csv file", b false, PortableSemantic),
        spec!(UseReplaceFile, "_Replace.csvを利用する", "Use _Replace.csv file", b true, PortableSemantic),
        spec!(UseMouse, "マウスを使用する", "Use mouse", b true, QueryOnlyClientPreference),
        spec!(UseMenu, "メニューを使用する", "Show menu", b true, QueryOnlyClientPreference),
        spec!(UseDebugCommand, "デバッグコマンドを使用する", "Allow debug commands", b false, PortableSemantic),
        spec!(AllowMultipleInstances, "多重起動を許可する", "Allow multiple instances", b true, UnsupportedPlatformIntegration),
        spec!(AutoSave, "オートセーブを行なう", "Make autosaves", b true, PortableSemantic),
        spec!(UseKeyMacro, "キーボードマクロを使用する", "Use keyboard macros", b true, PortableSemantic),
        spec!(SizableWindow, "ウィンドウの高さを可変にする", "Changeable window height", b true, QueryOnlyClientPreference),
        spec!(TextDrawingMode, "描画インターフェース", "Drawing interface", e "TEXTRENDERER" ["GRAPHICS", "TEXTRENDERER", "WINAPI"], QueryOnlyClientPreference),
        spec!(WindowX, "ウィンドウ幅", "Window width", i 760, QueryOnlyClientPreference),
        spec!(WindowY, "ウィンドウ高さ", "Window height", i 480, QueryOnlyClientPreference),
        spec!(WindowPosX, "ウィンドウ位置X", "Window X position", i 0, QueryOnlyClientPreference),
        spec!(WindowPosY, "ウィンドウ位置Y", "Window Y position", i 0, QueryOnlyClientPreference),
        spec!(SetWindowPos, "起動時のウィンドウ位置を指定する", "Fixed window starting position", b false, QueryOnlyClientPreference),
        spec!(WindowMaximixed, "起動時にウィンドウを最大化する", "Maximize window on startup", b false, QueryOnlyClientPreference),
        spec!(MaxLog, "履歴ログの行数", "Max history log lines", i 1000, PortableSemantic),
        spec!(PrintCPerLine, "PRINTCを並べる数", "Items per line for PRINTC", i 5, PortableSemantic),
        spec!(PrintCLength, "PRINTCの文字数", "Number of Item characters for PRINTC", i 24, PortableSemantic),
        spec!(FontName, "フォント名", "Font name", s "ＭＳ ゴシック", QueryOnlyClientPreference),
        spec!(FontSize, "フォントサイズ", "Font size", i 18, QueryOnlyClientPreference),
        spec!(LineHeight, "一行の高さ", "Line height", i 19, QueryOnlyClientPreference),
        spec!(ForeColor, "文字色", "Text color", c 0x00C0_C0C0, QueryOnlyClientPreference),
        spec!(BackColor, "背景色", "Background color", c 0, QueryOnlyClientPreference),
        spec!(FocusColor, "選択中文字色", "Highlight color", c 0x00FF_FF00, QueryOnlyClientPreference),
        spec!(LogColor, "履歴文字色", "History log color", c 0x00C0_C0C0, QueryOnlyClientPreference),
        spec!(FPS, "フレーム毎秒", "FPS", i 5, QueryOnlyClientPreference),
        spec!(SkipFrame, "最大スキップフレーム数", "Skip frames", i 3, QueryOnlyClientPreference),
        spec!(ScrollHeight, "スクロール行数", "Lines per scroll", i 1, QueryOnlyClientPreference),
        spec!(InfiniteLoopAlertTime, "無限ループ警告までのミリ秒数", "Milliseconds for infinite loop warning", i 5000, PortableSemantic),
        spec!(DisplayWarningLevel, "表示する最低警告レベル", "Minimum warning level", i 1, PortableSemantic),
        spec!(DisplayReport, "ロード時にレポートを表示する", "Display loading report", b false, QueryOnlyClientPreference),
        spec!(ReduceArgumentOnLoad, "ロード時に引数を解析する", "Reduce argument on load", e "NO" ["YES", "ONCE", "NO"], PortableSemantic),
        spec!(IgnoreUncalledFunction, "呼び出されなかった関数を無視する", "Ignore uncalled functions", b true, PortableSemantic),
        spec!(FunctionNotFoundWarning, "関数が見つからない警告の扱い", "Function is not found warning", e "IGNORE" ["IGNORE", "LATER", "ONCE", "DISPLAY"], PortableSemantic),
        spec!(FunctionNotCalledWarning, "関数が呼び出されなかった警告の扱い", "Function not called warning", e "IGNORE" ["IGNORE", "LATER", "ONCE", "DISPLAY"], PortableSemantic),
        spec!(ChangeMasterNameIfDebug, "デバッグコマンドを使用した時にMASTERの名前を変更する", "Change MASTER mame in debug", b true, PortableSemantic),
        spec!(ButtonWrap, "ボタンの途中で行を折りかえさない", "Button wrapping", b false, QueryOnlyClientPreference),
        spec!(SearchSubdirectory, "サブディレクトリを検索する", "Search subfolders", b false, PortableSemantic),
        spec!(SortWithFilename, "読み込み順をファイル名順にソートする", "Sort filenames", b false, PortableSemantic),
        spec!(LastKey, "最終更新コード", "Latest identify code", i 0, UnsupportedPlatformIntegration),
        spec!(SaveDataNos, "表示するセーブデータ数", "Save data count per page", i 20, PortableSemantic),
        spec!(WarnBackCompatibility, "eramaker互換性に関する警告を表示する", "Eramaker compatibility warning", b true, PortableSemantic),
        spec!(AllowFunctionOverloading, "システム関数の上書きを許可する", "Allow overriding system functions", b true, PortableSemantic),
        spec!(WarnFunctionOverloading, "システム関数が上書きされたとき警告を表示する", "System function override warning", b true, PortableSemantic),
        spec!(TextEditor, "関連づけるテキストエディタ", "Text editor", s "notepad", UnsupportedPlatformIntegration),
        spec!(EditorType, "テキストエディタコマンドライン指定", "Text editor command line setting", e "USER_SETTING" ["SAKURA", "TERAPAD", "EMEDITOR", "USER_SETTING"], UnsupportedPlatformIntegration),
        spec!(EditorArgument, "エディタに渡す行指定引数", "Text editor command line arguments", s "", UnsupportedPlatformIntegration),
        spec!(WarnNormalFunctionOverloading, "同名の非イベント関数が複数定義されたとき警告する", "Duplicated functions warning", b false, PortableSemantic),
        spec!(CompatiErrorLine, "解釈不可能な行があっても実行する", "Execute error lines", b false, PortableSemantic),
        spec!(CompatiCALLNAME, "CALLNAMEが空文字列の時にNAMEを代入する", "Use NAME if CALLNAME is empty", b false, PortableSemantic),
        spec!(UseSaveFolder, "セーブデータをsavフォルダ内に作成する", "Use sav folder", b false, QueryOnlyClientPreference),
        spec!(CompatiRAND, "擬似変数RANDの仕様をeramakerに合わせる", "Imitate behavior for RAND", b false, PortableSemantic),
        spec!(CompatiDRAWLINE, "DRAWLINEを常に新しい行で行う", "Always start DRAWLINE in a new line", b false, PortableSemantic),
        spec!(CompatiFunctionNoignoreCase, "関数・属性については大文字小文字を無視しない", "Do not ignore case for functions and attributes", b false, PortableSemantic),
        spec!(SystemAllowFullSpace, "全角スペースをホワイトスペースに含める", "Whitespace includes full-width space", b true, PortableSemantic),
        spec!(CompatiLinefeedAs1739, "ver1739以前の非ボタン折り返しを再現する", "Reproduce wrapping behavior like in pre ver1739", b false, PortableSemantic),
        spec!(useLanguage, "内部で使用する東アジア言語", "Default ANSI encoding", e "JAPANESE" ["JAPANESE", "KOREAN", "CHINESE_HANS", "CHINESE_HANT"], PortableSemantic),
        spec!(AllowLongInputByMouse, "ONEINPUT系命令でマウスによる2文字以上の入力を許可する", "Allow long input by mouse for ONEINPUT", b false, PortableSemantic),
        spec!(CompatiCallEvent, "イベント関数のCALLを許可する", "Allow CALL on event functions", b false, PortableSemantic),
        spec!(CompatiSPChara, "SPキャラを使用する", "Allow SP characters", b false, PortableSemantic),
        spec!(SystemSaveInBinary, "セーブデータをバイナリ形式で保存する", "Use the binary format for saving data", b false, PortableSemantic),
        spec!(CompatiFuncArgOptional, "ユーザー関数の全ての引数の省略を許可する", "Allow arguments omission for user functions", b false, PortableSemantic),
        spec!(CompatiFuncArgAutoConvert, "ユーザー関数の引数に自動的にTOSTRを補完する", "Auto TOSTR conversion for user function arguments", b false, PortableSemantic),
        spec!(SystemIgnoreTripleSymbol, "FORM中の三連記号を展開しない", "Do not process triple symbols inside FORM", b false, PortableSemantic),
        spec!(TimesNotRigorousCalculation, "TIMESの計算をeramakerにあわせる", "Imitate behavior for TIMES", b false, PortableSemantic),
        spec!(SystemNoTarget, "キャラクタ変数の引数を補完しない", "Do not auto-complete arguments for character variables", b false, PortableSemantic),
        spec!(SystemIgnoreStringSet, "文字列変数の代入に文字列式を強制する", "String variable assignment on valid with string expression", b false, PortableSemantic),
        spec!(ForbidUpdateCheck, "UPDATECHECKを許可しない", "Disallow UPDATECHECK", b false, PortableSemantic),
        spec!(UseERD, "ERD機能を利用する", "Use ERD", b true, PortableSemantic),
        spec!(VarsizeDimConfig, "VARSIZEの次元指定をERD機能に合わせる", "Imitate ERD to VARSIZE dimension specification", b false, PortableSemantic),
        spec!(CheckDuplicateIdentifier, "ERDで定義した識別子とローカル変数の重複を確認する", "Check duplicate ERD identifier and private variablea", b false, PortableSemantic),
        spec!(ReplaceContinuationBR, "行連結の改行コードの置換文字列", "String of replacing new line code inside continuation", s "\" \"", PortableSemantic),
        spec!(PluginAvailableWarn, "外部プラグインが有効時に警告を表示する", "If available pllugins, Show warning", b true, QueryOnlyClientPreference),
        spec!(
            ValidExtension,
            "LOADTEXTとSAVETEXTで使える拡張子",
            "Valid extensions for LOADTEXT and SAVETEXT",
            sl["txt"],
            PortableSemantic
        ),
        spec!(ZipSaveData, "セーブデータを圧縮して保存する", "Compress save data", b false, PortableSemantic),
        spec!(EnglishConfigOutput, "CONFIGファイルの内容を英語で保存する", "Output English items in the config file", b false, QueryOnlyClientPreference),
        spec!(EmueraLang, "Emueraの表示言語", "Emuera interface language", s "", QueryOnlyClientPreference),
        spec!(EmueraIcon, "Emueraのアイコンのパス", "Path to a custom window icon", s "", QueryOnlyClientPreference),
        spec!(CBUseClipboard, "表示したテキストをクリップボードにコピーする", "Clipboard- Copy text to Clipboard during Game", b false, UnsupportedPlatformIntegration),
        spec!(CBIgnoreTags, "テキスト中の<>タグを無視する", "Clipboard- ignore <> tags in text", b false, UnsupportedPlatformIntegration),
        spec!(CBReplaceTags, "<>を次の文で置き換える", "Clipboard- Replace <> with this", s ".", UnsupportedPlatformIntegration),
        spec!(CBNewLinesOnly, "新しい行のみコピーする", "Clipboard- Show new lines only", b true, UnsupportedPlatformIntegration),
        spec!(CBClearBuffer, "画面のリフレッシュ時にクリップボードとバッファを消去する", "Clipboard- Clear Buffer when game clears screen", b false, UnsupportedPlatformIntegration),
        spec!(CBTriggerLeftClick, "左クリックをトリガーにする", "Clipboard- LeftClick Trigger", b true, UnsupportedPlatformIntegration),
        spec!(CBTriggerMiddleClick, "ホイールクリックをトリガーにする", "Clipboard- MiddleClick Trigger", b false, UnsupportedPlatformIntegration),
        spec!(CBTriggerDoubleLeftClick, "ダブルクリックをトリガーにする", "Clipboard- Double Left Click Trigger", b false, UnsupportedPlatformIntegration),
        spec!(CBTriggerAnyKeyWait, "WAITをトリガーにする", "Clipboard- AnyKey Wait Trigger ", b false, UnsupportedPlatformIntegration),
        spec!(CBTriggerInputWait, "INPUTをトリガーにする", "Clipboard- Wait for Input Trigger", b true, UnsupportedPlatformIntegration),
        spec!(CBMaxCB, "クリップボードに貼り付ける行数", "Clipboard- Length of Clipboard", i 25, UnsupportedPlatformIntegration),
        spec!(CBBufferSize, "総バッファサイズ", "Clipboard- Buffer Size", i 300, UnsupportedPlatformIntegration),
        spec!(CBScrollCount, "スクロールの行数", "Clipboard- Scrolled Lines per Key", i 5, UnsupportedPlatformIntegration),
        spec!(CBMinTimer, "クリップボードの更新間隔(ミリ秒)", "Clipboard- min time between pastes", i 800, UnsupportedPlatformIntegration),
        spec!(RikaiEnabled, "Rikaichanを使用する", "Rikai- Enabled", b false, UnsupportedPlatformIntegration),
        spec!(RikaiFilename, "Rikaichanのファイルパス", "Rikai- Dictionary Filename", s "Emuera-Rikai-edict.txt-eucjp", UnsupportedPlatformIntegration),
        spec!(RikaiColorBack, "ポップアップの背景色", "Rikai- Back Color", c 0x0000_008B, UnsupportedPlatformIntegration),
        spec!(RikaiColorText, "ポップアップの文字色", "Rikai- Text Color", c 0x00FF_FFFF, UnsupportedPlatformIntegration),
        spec!(RikaiUseSeparateBoxes, "翻訳中の語句を強調表示する", "Rikai- Use Separate Boxes", b true, UnsupportedPlatformIntegration),
        spec!(Ctrl_Z_Enabled, "Ctrl-Zで元に戻す機能を有効にする", "Enable undo with ctrl-z", b false, PortableSemantic),
        spec!(DebugShowWindow, "起動時にデバッグウインドウを表示する", "Show debug window on startup", b true, QueryOnlyClientPreference),
        spec!(DebugWindowTopMost, "デバッグウインドウを最前面に表示する", "Debug window always on top", b true, QueryOnlyClientPreference),
        spec!(DebugWindowWidth, "デバッグウィンドウ幅", "Debug window width", i 400, QueryOnlyClientPreference),
        spec!(DebugWindowHeight, "デバッグウィンドウ高さ", "Debug window height", i 300, QueryOnlyClientPreference),
        spec!(DebugSetWindowPos, "デバッグウィンドウ位置を指定する", "Fixed debug window starting position", b false, QueryOnlyClientPreference),
        spec!(DebugWindowPosX, "デバッグウィンドウ位置X", "Debug window X position", i 0, QueryOnlyClientPreference),
        spec!(DebugWindowPosY, "デバッグウィンドウ位置Y", "Debug window Y position", i 0, QueryOnlyClientPreference),
        spec!(MoneyLabel, "お金の単位", "Currency symbol", s "$", PortableSemantic),
        spec!(MoneyFirst, "単位の位置", "Currency symbol position", b true, PortableSemantic),
        spec!(LoadLabel, "起動時簡略表示", "Loading message", s "Now Loading...", PortableSemantic),
        spec!(MaxShopItem, "販売アイテム数", "Max shop item storage", i 100, PortableSemantic),
        spec!(DrawLineString, "DRAWLINE文字", "DRAWLINE character", s "-", PortableSemantic),
        spec!(BarChar1, "BAR文字1", "BAR character 1", ch '*', PortableSemantic),
        spec!(BarChar2, "BAR文字2", "BAR character 2", ch '.', PortableSemantic),
        spec!(TitleMenuString0, "システムメニュー0", "System menu 0", s "最初からはじめる", PortableSemantic),
        spec!(TitleMenuString1, "システムメニュー1", "System menu 1", s "ロードしてはじめる", PortableSemantic),
        spec!(ComAbleDefault, "COM_ABLE初期値", "Default COM_ABLE", i 1, PortableSemantic),
        spec!(StainDefault, "汚れの初期値", "Default Stain", il [0, 0, 2, 1, 8], PortableSemantic),
        spec!(TimeupLabel, "時間切れ表示", "Time up message", s "時間切れ", PortableSemantic),
        spec!(ExpLvDef, "EXPLVの初期値", "Default EXPLV", il [0, 1, 4, 20, 50, 200], PortableSemantic),
        spec!(PalamLvDef, "PALAMLVの初期値", "Default PALAMLV", il [0, 100, 500, 3000, 10000, 30000, 60000, 100_000, 150_000, 250_000], PortableSemantic),
        spec!(pbandDef, "PBANDの初期値", "Default PBAND", i 4, PortableSemantic),
        spec!(RelationDef, "RELATIONの初期値", "Default RELATION", i 0, PortableSemantic),
        spec!(UseNewRandom, "新しい高速な乱数アルゴリズムを使う", "Use new random algorithm", b false, QueryOnlyClientPreference),
        spec!(AudioVolume, "ゲーム音量", "Game volume", i 100, QueryOnlyClientPreference),
        spec!(ReplaceFullWidthSpaces, "全角スペースを半角スペースに置換する", "Replace full-width spaces", b false, QueryOnlyClientPreference),
        spec!(CharacterWidthMode, "文字列幅計算モード", "Character width mode", e "AUTOMATIC" ["AUTOMATIC", "AMBIGUOUS_NARROW", "AMBIGUOUS_WIDE"], QueryOnlyClientPreference),
    ];
    for spec in &mut specs {
        spec.clients = clients(spec.code, spec.effect);
    }
    specs
}

fn clients(code: &str, effect: ConfigEffect) -> &'static [ConfigClient] {
    use ConfigClient::{Browser, Runtime, Tauri, Tui};
    let runtime = effect == ConfigEffect::PortableSemantic;
    match (
        runtime,
        tui_configurable(code),
        browser_configurable(code),
        tauri_configurable(code),
    ) {
        (false, false, false, false) => &[],
        (false, false, false, true) => &[Tauri],
        (false, false, true, false) => &[Browser],
        (false, false, true, true) => &[Browser, Tauri],
        (false, true, false, false) => &[Tui],
        (false, true, false, true) => &[Tui, Tauri],
        (false, true, true, false) => &[Tui, Browser],
        (false, true, true, true) => &[Tui, Browser, Tauri],
        (true, false, false, false) => &[Runtime],
        (true, false, false, true) => &[Runtime, Tauri],
        (true, false, true, false) => &[Runtime, Browser],
        (true, true, true, true) => &[Runtime, Tui, Browser, Tauri],
        (true, false, true, true) => &[Runtime, Browser, Tauri],
        (true, true, false, false) => &[Runtime, Tui],
        (true, true, false, true) => &[Runtime, Tui, Tauri],
        (true, true, true, false) => &[Runtime, Tui, Browser],
    }
}

/// Whether the Textual frontend exposes this effective setting.
#[must_use]
pub fn tui_configurable(code: &str) -> bool {
    tui_application(code).is_some()
}

/// Whether the browser frontend exposes this setting.
#[must_use]
pub fn browser_configurable(code: &str) -> bool {
    browser_application(code).is_some()
}

/// Whether the Tauri frontend exposes this setting.
#[must_use]
pub fn tauri_configurable(code: &str) -> bool {
    tauri_application(code).is_some()
}

/// Textual frontend defaults applied before project configuration files.
#[must_use]
pub fn tui_default(code: &str) -> Option<ConfigValue> {
    let _ = code;
    None
}

/// Browser and Tauri defaults applied before project configuration files.
#[must_use]
pub fn web_default(code: &str) -> Option<ConfigValue> {
    let _ = code;
    None
}

/// Application policy for settings visible in the Textual frontend.
#[must_use]
pub fn tui_application(code: &str) -> Option<ConfigApplication> {
    match code {
        "UseMouse"
        | "AllowLongInputByMouse"
        | "Ctrl_Z_Enabled"
        | "MaxLog"
        | "ButtonWrap"
        | "CompatiLinefeedAs1739"
        | "PrintCPerLine"
        | "PrintCLength"
        | "ForeColor"
        | "BackColor"
        | "FocusColor"
        | "ReplaceFullWidthSpaces"
        | "CharacterWidthMode" => Some(ConfigApplication::Hot),
        "UseRenameFile"
        | "UseReplaceFile"
        | "SearchSubdirectory"
        | "SortWithFilename"
        | "CompatiCALLNAME"
        | "CompatiSPChara"
        | "UseERD"
        | "VarsizeDimConfig"
        | "SystemAllowFullSpace"
        | "useLanguage"
        | "ReplaceContinuationBR"
        | "IgnoreCase"
        | "IgnoreUncalledFunction"
        | "AllowFunctionOverloading"
        | "WarnFunctionOverloading"
        | "DisplayWarningLevel"
        | "FunctionNotFoundWarning"
        | "FunctionNotCalledWarning"
        | "CompatiCallEvent"
        | "CompatiFuncArgOptional"
        | "CompatiFuncArgAutoConvert"
        | "SystemIgnoreTripleSymbol"
        | "AutoSave"
        | "SaveDataNos"
        | "SystemSaveInBinary"
        | "ZipSaveData"
        | "EnglishConfigOutput" => Some(ConfigApplication::Restart),
        _ => None,
    }
}

/// Application policy shared by browser and Tauri settings.
#[must_use]
pub fn browser_application(code: &str) -> Option<ConfigApplication> {
    match code {
        "UseMenu"
        | "UseMouse"
        | "AllowLongInputByMouse"
        | "Ctrl_Z_Enabled"
        | "ScrollHeight"
        | "MaxLog"
        | "ButtonWrap"
        | "CompatiLinefeedAs1739"
        | "PrintCPerLine"
        | "PrintCLength"
        | "FontName"
        | "FontSize"
        | "LineHeight"
        | "ForeColor"
        | "BackColor"
        | "FocusColor" => Some(ConfigApplication::Hot),
        "AudioVolume" | "ReplaceFullWidthSpaces" | "CharacterWidthMode" => {
            Some(ConfigApplication::Hot)
        }
        "UseRenameFile"
        | "UseReplaceFile"
        | "SearchSubdirectory"
        | "SortWithFilename"
        | "CompatiCALLNAME"
        | "CompatiSPChara"
        | "UseERD"
        | "VarsizeDimConfig"
        | "SystemAllowFullSpace"
        | "useLanguage"
        | "ReplaceContinuationBR"
        | "IgnoreCase"
        | "IgnoreUncalledFunction"
        | "AllowFunctionOverloading"
        | "WarnFunctionOverloading"
        | "DisplayWarningLevel"
        | "FunctionNotFoundWarning"
        | "FunctionNotCalledWarning"
        | "CompatiCallEvent"
        | "CompatiFuncArgOptional"
        | "CompatiFuncArgAutoConvert"
        | "SystemIgnoreTripleSymbol"
        | "AutoSave"
        | "SaveDataNos"
        | "SystemSaveInBinary"
        | "ZipSaveData"
        | "EnglishConfigOutput" => Some(ConfigApplication::Restart),
        _ => None,
    }
}

/// Application policy for settings exposed by the Tauri frontend.
#[must_use]
pub fn tauri_application(code: &str) -> Option<ConfigApplication> {
    match code {
        "WindowMaximixed" | "WindowX" | "WindowY" => Some(ConfigApplication::Hot),
        _ => browser_application(code),
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ConfigStore {
    values: BTreeMap<String, ConfigValue>,
    fixed: BTreeMap<String, bool>,
}

impl Default for ConfigStore {
    fn default() -> Self {
        let values = catalog()
            .into_iter()
            .map(|spec| (spec.code.to_ascii_uppercase(), spec.default))
            .collect();
        Self {
            values,
            fixed: BTreeMap::new(),
        }
    }
}

impl ConfigStore {
    /// Construct the catalog with Textual-specific defaults before project files apply.
    #[must_use]
    pub fn with_tui_defaults() -> Self {
        let mut store = Self::default();
        for spec in catalog() {
            if let Some(value) = tui_default(spec.code) {
                store.values.insert(spec.code.to_ascii_uppercase(), value);
            }
        }
        store
    }

    /// Construct the catalog with browser/Tauri defaults before project files apply.
    #[must_use]
    pub fn with_web_defaults() -> Self {
        let mut store = Self::default();
        for spec in catalog() {
            if let Some(value) = web_default(spec.code) {
                store.values.insert(spec.code.to_ascii_uppercase(), value);
            }
        }
        store
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&ConfigValue> {
        let code = resolve_code(name)?;
        self.values.get(&code)
    }

    #[must_use]
    pub fn get_code(&self, code: &str) -> Option<&ConfigValue> {
        self.values.get(&code.to_ascii_uppercase())
    }

    #[must_use]
    pub fn is_fixed(&self, code: &str) -> bool {
        self.fixed
            .get(&code.to_ascii_uppercase())
            .copied()
            .unwrap_or(false)
    }

    /// Apply one `name:value` assignment. Unknown keys and invalid values are rejected.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigParseError`] when the key is unknown or the value does not
    /// match the catalog entry's type.
    pub fn apply(&mut self, name: &str, raw: &str, fixed: bool) -> Result<(), ConfigParseError> {
        let code = resolve_code(name).ok_or(ConfigParseError::UnknownKey)?;
        if self.fixed.get(&code).copied().unwrap_or(false) {
            return Ok(());
        }
        let current = self.values.get(&code).ok_or(ConfigParseError::UnknownKey)?;
        let parsed = parse_like(&code, current, raw)?;
        self.values.insert(code.clone(), parsed);
        if fixed {
            self.fixed.insert(code, true);
        }
        Ok(())
    }

    /// Apply an emuera/default/fixed config assignment. Replace and debug items live
    /// in different reference files and therefore are not accepted here.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigParseError`] for an unknown, replace/debug-only, or malformed
    /// assignment.
    pub fn apply_regular(
        &mut self,
        name: &str,
        raw: &str,
        fixed: bool,
    ) -> Result<(), ConfigParseError> {
        let code = resolve_code(name).ok_or(ConfigParseError::UnknownKey)?;
        if !is_regular_code(&code) {
            return Err(ConfigParseError::UnknownKey);
        }
        self.apply(&code, raw, fixed)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &ConfigValue)> {
        self.values.iter().map(|(key, value)| (key.as_str(), value))
    }

    /// Serialize the complete regular configuration in the pinned reference order.
    #[must_use]
    pub fn serialize_regular(&self) -> String {
        let english = matches!(
            self.get_code("EnglishConfigOutput"),
            Some(ConfigValue::Boolean(true))
        );
        let mut output = String::new();
        for spec in catalog() {
            let code = spec.code.to_ascii_uppercase();
            if !is_regular_code(&code) {
                continue;
            }
            let Some(value) = self.values.get(&code) else {
                continue;
            };
            output.push_str(if english { spec.english } else { spec.japanese });
            output.push(':');
            output.push_str(&value.config_text());
            output.push('\n');
        }
        output
    }
}

#[must_use]
pub fn is_regular_code(code: &str) -> bool {
    let code = code.to_ascii_uppercase();
    !is_replace_code(&code)
        && !code.starts_with("DEBUG")
        && !matches!(
            code.as_str(),
            "USENEWRANDOM" | "AUDIOVOLUME" | "REPLACEFULLWIDTHSPACES" | "CHARACTERWIDTHMODE"
        )
}

pub(crate) fn is_replace_code(code: &str) -> bool {
    matches!(
        code,
        "MONEYLABEL"
            | "MONEYFIRST"
            | "LOADLABEL"
            | "MAXSHOPITEM"
            | "DRAWLINESTRING"
            | "BARCHAR1"
            | "BARCHAR2"
            | "TITLEMENUSTRING0"
            | "TITLEMENUSTRING1"
            | "COMABLEDEFAULT"
            | "STAINDEFAULT"
            | "TIMEUPLABEL"
            | "EXPLVDEF"
            | "PALAMLVDEF"
            | "PBANDDEF"
            | "RELATIONDEF"
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigParseError {
    UnknownKey,
    InvalidValue,
}

pub(crate) fn resolve_code(name: &str) -> Option<String> {
    let key = name.trim().to_uppercase();
    catalog()
        .into_iter()
        .find(|spec| {
            spec.code.to_ascii_uppercase() == key
                || spec.japanese.to_uppercase() == key
                || spec.english.to_uppercase() == key
        })
        .map(|spec| spec.code.to_ascii_uppercase())
}

fn parse_like(
    code: &str,
    current: &ConfigValue,
    raw: &str,
) -> Result<ConfigValue, ConfigParseError> {
    if raw.is_empty() {
        return Err(ConfigParseError::InvalidValue);
    }
    let value = raw.trim();
    match current {
        ConfigValue::Boolean(_) => {
            if let Ok(value) = value.parse::<i32>() {
                return Ok(ConfigValue::Boolean(value != 0));
            }
            match value.to_ascii_uppercase().as_str() {
                "YES" | "TRUE" | "前" => Ok(ConfigValue::Boolean(true)),
                "NO" | "FALSE" | "後" => Ok(ConfigValue::Boolean(false)),
                _ => Err(ConfigParseError::InvalidValue),
            }
        }
        ConfigValue::Integer(_) => if matches!(code, "LASTKEY" | "PBANDDEF" | "RELATIONDEF") {
            value.parse::<i64>()
        } else {
            value.parse::<i32>().map(i64::from)
        }
        .map(ConfigValue::Integer)
        .map_err(|_| ConfigParseError::InvalidValue),
        ConfigValue::String(_) => Ok(ConfigValue::String(value.into())),
        ConfigValue::Enum { allowed, .. } => {
            let parsed = allowed
                .iter()
                .find(|candidate| candidate.eq_ignore_ascii_case(value))
                .cloned()
                .or_else(|| {
                    value.parse::<i32>().ok().map(|ordinal| {
                        usize::try_from(ordinal)
                            .ok()
                            .and_then(|index| allowed.get(index).cloned())
                            .unwrap_or_else(|| ordinal.to_string())
                    })
                })
                .ok_or(ConfigParseError::InvalidValue)?;
            Ok(ConfigValue::Enum {
                value: parsed,
                allowed: allowed.clone(),
            })
        }
        ConfigValue::Color(_) => {
            let parts = value
                .split(',')
                .take(3)
                .map(str::trim)
                .map(str::parse::<u8>)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| ConfigParseError::InvalidValue)?;
            let [r, g, b, ..] = parts.as_slice() else {
                return Err(ConfigParseError::InvalidValue);
            };
            Ok(ConfigValue::Color(
                (u32::from(*r) << 16) | (u32::from(*g) << 8) | u32::from(*b),
            ))
        }
        ConfigValue::Character(_) => {
            let mut characters = value.chars();
            let character = characters.next().ok_or(ConfigParseError::InvalidValue)?;
            if characters.next().is_some() {
                return Err(ConfigParseError::InvalidValue);
            }
            Ok(ConfigValue::Character(character))
        }
        ConfigValue::IntegerList(_) => value
            .split('/')
            .map(str::trim)
            .map(str::parse::<i64>)
            .collect::<Result<Vec<_>, _>>()
            .map(ConfigValue::IntegerList)
            .map_err(|_| ConfigParseError::InvalidValue),
        ConfigValue::StringList(_) => Ok(ConfigValue::StringList(
            value.split(',').map(str::trim).map(Into::into).collect(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn catalog_aliases_types_and_fixed_precedence_are_deterministic() {
        assert_eq!(catalog().len(), 127, "Era configuration catalog drifted");
        let mut store = ConfigStore::default();
        assert_eq!(
            store.get("描画インターフェース"),
            Some(&ConfigValue::Enum {
                value: "TEXTRENDERER".into(),
                allowed: vec!["GRAPHICS".into(), "TEXTRENDERER".into(), "WINAPI".into()],
            })
        );
        assert_eq!(store.get("Drawing interface"), store.get("TextDrawingMode"));
        assert_eq!(
            store.get("フォントサイズ").unwrap().script_value(),
            ScriptConfigValue::Integer(18)
        );
        assert_eq!(
            store.get("文字色").unwrap().script_value(),
            ScriptConfigValue::Integer(0x00C0_C0C0)
        );
        assert_eq!(
            store.get("汚れの初期値").unwrap().script_value(),
            ScriptConfigValue::String("System.Collections.Generic.List`1[System.Int64]".into())
        );
        store.apply("Text color", "1,2,3", false).unwrap();
        assert_eq!(
            store.get("文字色").unwrap().script_value(),
            ScriptConfigValue::Integer(0x0001_0203)
        );
        store.apply("Font size", "21", true).unwrap();
        store.apply("Font size", "99", false).unwrap();
        assert_eq!(store.get("FontSize"), Some(&ConfigValue::Integer(21)));
        store.apply("Make autosaves", "-2", false).unwrap();
        assert_eq!(store.get("AutoSave"), Some(&ConfigValue::Boolean(true)));
        store.apply("Drawing interface", "2", false).unwrap();
        assert_eq!(
            store.get("TextDrawingMode").unwrap().script_value(),
            ScriptConfigValue::String("WINAPI".into())
        );
        store.apply("BAR character 1", "β", false).unwrap();
        assert_eq!(store.get("BarChar1"), Some(&ConfigValue::Character('β')));
        let keys = store.iter().map(|(key, _)| key).collect::<Vec<_>>();
        assert!(keys.windows(2).all(|pair| pair[0] <= pair[1]));
        let font = catalog()
            .into_iter()
            .find(|spec| spec.code == "FontSize")
            .unwrap();
        assert_eq!(font.clients, &[ConfigClient::Browser, ConfigClient::Tauri]);
        let editor = catalog()
            .into_iter()
            .find(|spec| spec.code == "TextEditor")
            .unwrap();
        assert!(editor.clients.is_empty());
    }

    #[test]
    fn tui_profile_has_exact_surface_defaults_and_application_policies() {
        let exposed = catalog()
            .into_iter()
            .filter(|spec| spec.clients.contains(&ConfigClient::Tui))
            .map(|spec| spec.code)
            .collect::<BTreeSet<_>>();
        let expected = [
            "AllowFunctionOverloading",
            "AllowLongInputByMouse",
            "AutoSave",
            "BackColor",
            "ButtonWrap",
            "CompatiCALLNAME",
            "CompatiCallEvent",
            "CompatiFuncArgAutoConvert",
            "CompatiFuncArgOptional",
            "CompatiLinefeedAs1739",
            "CompatiSPChara",
            "Ctrl_Z_Enabled",
            "DisplayWarningLevel",
            "EnglishConfigOutput",
            "FocusColor",
            "ForeColor",
            "FunctionNotCalledWarning",
            "FunctionNotFoundWarning",
            "IgnoreCase",
            "IgnoreUncalledFunction",
            "MaxLog",
            "PrintCLength",
            "PrintCPerLine",
            "ReplaceFullWidthSpaces",
            "ReplaceContinuationBR",
            "SaveDataNos",
            "SearchSubdirectory",
            "SortWithFilename",
            "SystemAllowFullSpace",
            "SystemIgnoreTripleSymbol",
            "SystemSaveInBinary",
            "UseERD",
            "UseMouse",
            "UseRenameFile",
            "UseReplaceFile",
            "VarsizeDimConfig",
            "WarnFunctionOverloading",
            "ZipSaveData",
            "CharacterWidthMode",
            "useLanguage",
        ]
        .into_iter()
        .collect::<BTreeSet<_>>();
        assert_eq!(exposed, expected);

        let store = ConfigStore::with_tui_defaults();
        assert_eq!(store.get_code("MaxLog"), Some(&ConfigValue::Integer(1_000)));
        assert_eq!(
            store.get_code("PrintCPerLine"),
            Some(&ConfigValue::Integer(5))
        );
        assert_eq!(
            store.get_code("PrintCLength"),
            Some(&ConfigValue::Integer(24))
        );
        assert_eq!(
            expected
                .iter()
                .filter(|code| tui_application(code) == Some(ConfigApplication::Hot))
                .count(),
            13
        );
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn browser_and_tauri_profiles_have_exact_surfaces_defaults_and_policies() {
        for spec in catalog() {
            assert_eq!(
                spec.clients.contains(&ConfigClient::Tui),
                tui_application(spec.code).is_some(),
                "TUI surface and policy drifted for {}",
                spec.code
            );
            assert_eq!(
                spec.clients.contains(&ConfigClient::Browser),
                browser_application(spec.code).is_some(),
                "browser surface and policy drifted for {}",
                spec.code
            );
            assert_eq!(
                spec.clients.contains(&ConfigClient::Tauri),
                tauri_application(spec.code).is_some(),
                "Tauri surface and policy drifted for {}",
                spec.code
            );
        }
        let browser = catalog()
            .into_iter()
            .filter(|spec| spec.clients.contains(&ConfigClient::Browser))
            .map(|spec| spec.code)
            .collect::<BTreeSet<_>>();
        let expected = [
            "AllowFunctionOverloading",
            "AllowLongInputByMouse",
            "AudioVolume",
            "AutoSave",
            "BackColor",
            "ButtonWrap",
            "CompatiCALLNAME",
            "CompatiCallEvent",
            "CompatiFuncArgAutoConvert",
            "CompatiFuncArgOptional",
            "CompatiLinefeedAs1739",
            "CompatiSPChara",
            "Ctrl_Z_Enabled",
            "DisplayWarningLevel",
            "EnglishConfigOutput",
            "FocusColor",
            "FontName",
            "FontSize",
            "ForeColor",
            "FunctionNotCalledWarning",
            "FunctionNotFoundWarning",
            "IgnoreCase",
            "IgnoreUncalledFunction",
            "LineHeight",
            "MaxLog",
            "PrintCLength",
            "PrintCPerLine",
            "ReplaceFullWidthSpaces",
            "ReplaceContinuationBR",
            "SaveDataNos",
            "ScrollHeight",
            "SearchSubdirectory",
            "SortWithFilename",
            "SystemAllowFullSpace",
            "SystemIgnoreTripleSymbol",
            "SystemSaveInBinary",
            "UseERD",
            "UseMenu",
            "UseMouse",
            "UseRenameFile",
            "UseReplaceFile",
            "VarsizeDimConfig",
            "WarnFunctionOverloading",
            "ZipSaveData",
            "CharacterWidthMode",
            "useLanguage",
        ]
        .into_iter()
        .collect::<BTreeSet<_>>();
        assert_eq!(browser, expected);

        let tauri = catalog()
            .into_iter()
            .filter(|spec| spec.clients.contains(&ConfigClient::Tauri))
            .map(|spec| spec.code)
            .collect::<BTreeSet<_>>();
        let mut expected_tauri = expected;
        expected_tauri.extend(["WindowMaximixed", "WindowX", "WindowY"]);
        assert_eq!(tauri, expected_tauri);
        assert_eq!(
            expected_tauri
                .iter()
                .filter(|code| tauri_application(code) == Some(ConfigApplication::Hot))
                .count(),
            22
        );

        let store = ConfigStore::with_web_defaults();
        assert_eq!(store.get_code("MaxLog"), Some(&ConfigValue::Integer(1_000)));
        assert_eq!(
            store.get_code("PrintCPerLine"),
            Some(&ConfigValue::Integer(5))
        );
        assert_eq!(
            store.get_code("PrintCLength"),
            Some(&ConfigValue::Integer(24))
        );
        for code in ["SizableWindow", "SetWindowPos", "WindowPosX", "WindowPosY"] {
            assert!(!tauri_configurable(code));
        }
    }

    #[test]
    fn regular_configuration_serialization_is_canonical_and_respects_language() {
        let mut store = ConfigStore::default();
        store.apply_regular("Font size", "22", false).unwrap();
        store.apply_regular("Text color", "1,2,3", false).unwrap();
        let japanese = store.serialize_regular();
        assert!(japanese.contains("フォントサイズ:22\n"));
        assert!(japanese.contains("文字色:1,2,3\n"));
        assert!(!japanese.contains("デバッグウインドウ幅"));
        assert!(!japanese.contains("DRAWLINE文字"));

        store
            .apply_regular("EnglishConfigOutput", "YES", false)
            .unwrap();
        let english = store.serialize_regular();
        assert!(english.contains("Font size:22\n"));
        assert!(english.contains("Text color:1,2,3\n"));
        assert!(english.contains("Output English items in the config file:YES\n"));
    }
}
