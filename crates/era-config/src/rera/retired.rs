pub(super) struct RetiredConfigSpec {
    pub(super) id: u16,
    pub(super) code: &'static str,
    pub(super) path: &'static str,
    pub(super) japanese: &'static str,
    pub(super) english: &'static str,
}

pub(super) const RETIRED_CONFIG_SPECS: &[RetiredConfigSpec] = &[
    retired(
        11,
        "TextDrawingMode",
        "text.drawing_method",
        "描画インターフェース",
        "Drawing interface",
    ),
    retired(
        29,
        "SkipFrame",
        "display.maximum_skipped_frames",
        "最大スキップフレーム数",
        "Skip frames",
    ),
    retired(
        33,
        "DisplayReport",
        "project.show_loading_report",
        "ロード時にレポートを表示する",
        "Display loading report",
    ),
    retired(
        34,
        "ReduceArgumentOnLoad",
        "project.reduce_arguments_on_load",
        "ロード時に引数を解析する",
        "Reduce argument on load",
    ),
    retired(
        42,
        "LastKey",
        "project.last_update_code",
        "最終更新コード",
        "Latest identify code",
    ),
    retired(
        47,
        "TextEditor",
        "editor.command",
        "関連づけるテキストエディタ",
        "Text editor",
    ),
    retired(
        48,
        "EditorType",
        "editor.argument_style",
        "テキストエディタコマンドライン指定",
        "Text editor command line setting",
    ),
    retired(
        49,
        "EditorArgument",
        "editor.arguments",
        "エディタに渡す行指定引数",
        "Text editor command line arguments",
    ),
    retired(
        53,
        "UseSaveFolder",
        "save.use_sav_directory",
        "セーブデータをsavフォルダ内に作成する",
        "Use sav folder",
    ),
    retired(
        55,
        "CompatiDRAWLINE",
        "compatibility.drawline_starts_new_line",
        "DRAWLINEを常に新しい行で行う",
        "Always start DRAWLINE in a new line",
    ),
    retired(
        56,
        "CompatiFunctionNoignoreCase",
        "compatibility.function_names_case_sensitive",
        "関数・属性については大文字小文字を無視しない",
        "Do not ignore case for functions and attributes",
    ),
    retired(
        78,
        "EnglishConfigOutput",
        "legacy.write_english_config_keys",
        "CONFIGファイルの内容を英語で保存する",
        "Output English items in the config file",
    ),
    retired(
        79,
        "EmueraLang",
        "interface.language",
        "Emueraの表示言語",
        "Emuera interface language",
    ),
    retired(
        81,
        "CBUseClipboard",
        "clipboard.enabled",
        "表示したテキストをクリップボードにコピーする",
        "Clipboard- Copy text to Clipboard during Game",
    ),
    retired(
        82,
        "CBIgnoreTags",
        "clipboard.strip_angle_bracket_tags",
        "テキスト中の<>タグを無視する",
        "Clipboard- ignore <> tags in text",
    ),
    retired(
        83,
        "CBReplaceTags",
        "clipboard.tag_replacement",
        "<>を次の文で置き換える",
        "Clipboard- Replace <> with this",
    ),
    retired(
        84,
        "CBNewLinesOnly",
        "clipboard.new_lines_only",
        "新しい行のみコピーする",
        "Clipboard- Show new lines only",
    ),
    retired(
        85,
        "CBClearBuffer",
        "clipboard.clear_on_screen_refresh",
        "画面のリフレッシュ時にクリップボードとバッファを消去する",
        "Clipboard- Clear Buffer when game clears screen",
    ),
    retired(
        86,
        "CBTriggerLeftClick",
        "clipboard.trigger_left_click",
        "左クリックをトリガーにする",
        "Clipboard- LeftClick Trigger",
    ),
    retired(
        87,
        "CBTriggerMiddleClick",
        "clipboard.trigger_middle_click",
        "ホイールクリックをトリガーにする",
        "Clipboard- MiddleClick Trigger",
    ),
    retired(
        88,
        "CBTriggerDoubleLeftClick",
        "clipboard.trigger_double_click",
        "ダブルクリックをトリガーにする",
        "Clipboard- Double Left Click Trigger",
    ),
    retired(
        89,
        "CBTriggerAnyKeyWait",
        "clipboard.trigger_wait",
        "WAITをトリガーにする",
        "Clipboard- AnyKey Wait Trigger",
    ),
    retired(
        90,
        "CBTriggerInputWait",
        "clipboard.trigger_input",
        "INPUTをトリガーにする",
        "Clipboard- Wait for Input Trigger",
    ),
    retired(
        91,
        "CBMaxCB",
        "clipboard.lines_per_copy",
        "クリップボードに貼り付ける行数",
        "Clipboard- Length of Clipboard",
    ),
    retired(
        92,
        "CBBufferSize",
        "clipboard.buffer_lines",
        "総バッファサイズ",
        "Clipboard- Buffer Size",
    ),
    retired(
        93,
        "CBScrollCount",
        "clipboard.scroll_lines",
        "スクロールの行数",
        "Clipboard- Scrolled Lines per Key",
    ),
    retired(
        94,
        "CBMinTimer",
        "clipboard.minimum_interval_ms",
        "クリップボードの更新間隔(ミリ秒)",
        "Clipboard- min time between pastes",
    ),
    retired(
        95,
        "RikaiEnabled",
        "dictionary.enabled",
        "Rikaichanを使用する",
        "Rikai- Enabled",
    ),
    retired(
        96,
        "RikaiFilename",
        "dictionary.file_path",
        "Rikaichanのファイルパス",
        "Rikai- Dictionary Filename",
    ),
    retired(
        97,
        "RikaiColorBack",
        "dictionary.background_color",
        "ポップアップの背景色",
        "Rikai- Back Color",
    ),
    retired(
        98,
        "RikaiColorText",
        "dictionary.text_color",
        "ポップアップの文字色",
        "Rikai- Text Color",
    ),
    retired(
        99,
        "RikaiUseSeparateBoxes",
        "dictionary.highlight_current_phrase",
        "翻訳中の語句を強調表示する",
        "Rikai- Use Separate Boxes",
    ),
    retired(
        101,
        "DebugShowWindow",
        "debug_window.show_on_start",
        "起動時にデバッグウインドウを表示する",
        "Show debug window on startup",
    ),
    retired(
        102,
        "DebugWindowTopMost",
        "debug_window.always_on_top",
        "デバッグウインドウを最前面に表示する",
        "Debug window always on top",
    ),
    retired(
        103,
        "DebugWindowWidth",
        "debug_window.width",
        "デバッグウィンドウ幅",
        "Debug window width",
    ),
    retired(
        104,
        "DebugWindowHeight",
        "debug_window.height",
        "デバッグウィンドウ高さ",
        "Debug window height",
    ),
    retired(
        105,
        "DebugSetWindowPos",
        "debug_window.use_saved_position",
        "デバッグウィンドウ位置を指定する",
        "Fixed debug window starting position",
    ),
    retired(
        106,
        "DebugWindowPosX",
        "debug_window.position_x",
        "デバッグウィンドウ位置X",
        "Debug window X position",
    ),
    retired(
        107,
        "DebugWindowPosY",
        "debug_window.position_y",
        "デバッグウィンドウ位置Y",
        "Debug window Y position",
    ),
    retired(
        124,
        "UseNewRandom",
        "legacy.use_new_random",
        "新しい高速な乱数アルゴリズムを使う",
        "Use new random algorithm",
    ),
];

const fn retired(
    id: u16,
    code: &'static str,
    path: &'static str,
    japanese: &'static str,
    english: &'static str,
) -> RetiredConfigSpec {
    RetiredConfigSpec {
        id,
        code,
        path,
        japanese,
        english,
    }
}

pub(super) fn retired_by_path(path: &str) -> Option<&'static RetiredConfigSpec> {
    RETIRED_CONFIG_SPECS.iter().find(|spec| spec.path == path)
}

pub(super) fn resolve_retired_code(name: &str) -> Option<&'static str> {
    let name = name.trim();
    RETIRED_CONFIG_SPECS
        .iter()
        .find(|spec| {
            spec.code.eq_ignore_ascii_case(name)
                || spec.japanese.eq_ignore_ascii_case(name)
                || spec.english.eq_ignore_ascii_case(name)
        })
        .map(|spec| spec.code)
}
