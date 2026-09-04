use crate::ConfigValue;
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

/// Return the supported project-setting catalog derived from the pinned `ConfigData.setDefault`.
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
        spec!(UseMenu, "メニューを使用する", "Show menu", e "AUTO" ["SHOW", "AUTO", "HIDE"], QueryOnlyClientPreference),
        spec!(UseDebugCommand, "デバッグコマンドを使用する", "Allow debug commands", b false, PortableSemantic),
        spec!(AllowMultipleInstances, "多重起動を許可する", "Allow multiple instances", b true, UnsupportedPlatformIntegration),
        spec!(AutoSave, "オートセーブを行なう", "Make autosaves", b true, PortableSemantic),
        spec!(UseKeyMacro, "キーボードマクロを使用する", "Use keyboard macros", b true, PortableSemantic),
        spec!(SizableWindow, "ウィンドウの高さを可変にする", "Changeable window height", b true, QueryOnlyClientPreference),
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
        spec!(ScrollHeight, "スクロール行数", "Lines per scroll", i 1, QueryOnlyClientPreference),
        spec!(InfiniteLoopAlertTime, "無限ループ警告までのミリ秒数", "Milliseconds for infinite loop warning", i 5000, PortableSemantic),
        spec!(DisplayWarningLevel, "表示する最低警告レベル", "Minimum warning level", i 1, PortableSemantic),
        spec!(StrictUserCallArguments, "蛇版ユーザー関数の過剰引数をエラーにする", "Treat snake excess user arguments as errors", b false, PortableSemantic),
        spec!(DisableBeforeErrorThrow, "BEFORE_ERROR/THROWイベントを無効化する", "Disable BEFORE_ERROR/THROW events", b false, PortableSemantic),
        spec!(IgnoreUncalledFunction, "呼び出されなかった関数を無視する", "Ignore uncalled functions", b true, PortableSemantic),
        spec!(FunctionNotFoundWarning, "関数が見つからない警告の扱い", "Function is not found warning", e "IGNORE" ["IGNORE", "LATER", "ONCE", "DISPLAY"], PortableSemantic),
        spec!(FunctionNotCalledWarning, "関数が呼び出されなかった警告の扱い", "Function not called warning", e "IGNORE" ["IGNORE", "LATER", "ONCE", "DISPLAY"], PortableSemantic),
        spec!(ChangeMasterNameIfDebug, "デバッグコマンドを使用した時にMASTERの名前を変更する", "Change MASTER mame in debug", b true, PortableSemantic),
        spec!(ButtonWrap, "ボタンの途中で行を折りかえさない", "Button wrapping", b false, QueryOnlyClientPreference),
        spec!(SearchSubdirectory, "サブディレクトリを検索する", "Search subfolders", b false, PortableSemantic),
        spec!(SortWithFilename, "読み込み順をファイル名順にソートする", "Sort filenames", b false, PortableSemantic),
        spec!(SaveDataNos, "表示するセーブデータ数", "Save data count per page", i 20, PortableSemantic),
        spec!(WarnBackCompatibility, "eramaker互換性に関する警告を表示する", "Eramaker compatibility warning", b true, PortableSemantic),
        spec!(AllowFunctionOverloading, "システム関数の上書きを許可する", "Allow overriding system functions", b true, PortableSemantic),
        spec!(WarnFunctionOverloading, "システム関数が上書きされたとき警告を表示する", "System function override warning", b true, PortableSemantic),
        spec!(WarnNormalFunctionOverloading, "同名の非イベント関数が複数定義されたとき警告する", "Duplicated functions warning", b false, PortableSemantic),
        spec!(CompatiErrorLine, "解釈不可能な行があっても実行する", "Execute error lines", b false, PortableSemantic),
        spec!(CompatiCALLNAME, "CALLNAMEが空文字列の時にNAMEを代入する", "Use NAME if CALLNAME is empty", b false, PortableSemantic),
        spec!(CompatiRAND, "擬似変数RANDの仕様をeramakerに合わせる", "Imitate behavior for RAND", b false, PortableSemantic),
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
        spec!(EmueraIcon, "Emueraのアイコンのパス", "Path to a custom window icon", s "", QueryOnlyClientPreference),
        spec!(Ctrl_Z_Enabled, "Ctrl-Zで元に戻す機能を有効にする", "Enable undo with ctrl-z", b false, PortableSemantic),
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
        spec!(AudioVolume, "ゲーム音量", "Game volume", i 100, QueryOnlyClientPreference),
        spec!(ReplaceFullWidthSpaces, "全角スペースを半角スペースに置換する", "Replace full-width spaces", b false, QueryOnlyClientPreference),
        spec!(CharacterWidthMode, "文字列幅計算モード", "Character width mode", e "AUTOMATIC" ["AUTOMATIC", "AMBIGUOUS_NARROW", "AMBIGUOUS_WIDE"], PortableSemantic),
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
pub fn tui_default(_code: &str) -> Option<ConfigValue> {
    None
}

/// Browser and Tauri defaults applied before project configuration files.
#[must_use]
pub fn web_default(_code: &str) -> Option<ConfigValue> {
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
        _ => shared_restart_application(code),
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
        _ => shared_restart_application(code),
    }
}

fn shared_restart_application(code: &str) -> Option<ConfigApplication> {
    match code {
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
        | "StrictUserCallArguments"
        | "DisableBeforeErrorThrow"
        | "FunctionNotFoundWarning"
        | "FunctionNotCalledWarning"
        | "CompatiCallEvent"
        | "CompatiFuncArgOptional"
        | "CompatiFuncArgAutoConvert"
        | "SystemIgnoreTripleSymbol"
        | "AutoSave"
        | "SaveDataNos"
        | "SystemSaveInBinary"
        | "ZipSaveData" => Some(ConfigApplication::Restart),
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
