#[cfg(test)]
use std::collections::BTreeSet;

use toml_edit::{Array, Item, Value};

use crate::{ConfigValue, catalog};

use super::{
    ByteSpan, ReraConfigError, ReraConfigErrorKind, ReraConfigSpec, error_at,
    retired::RETIRED_CONFIG_SPECS,
};

#[must_use]
pub fn rera_catalog() -> Vec<ReraConfigSpec> {
    let specs = catalog()
        .into_iter()
        .map(|spec| {
            let (id, path, description_zh_cn, deprecated) = metadata(spec.code);
            let (minimum, maximum) = integer_bounds(spec.code, &spec.default);
            ReraConfigSpec {
                id,
                code: spec.code,
                path,
                description_zh_cn,
                default: spec.default,
                clients: spec.clients,
                minimum,
                maximum,
                deprecated,
            }
        })
        .collect::<Vec<_>>();
    debug_assert!(RETIRED_CONFIG_SPECS.iter().all(|retired| {
        specs
            .iter()
            .all(|active| active.id != retired.id && active.code != retired.code)
    }));
    specs
}

#[cfg(test)]
pub(super) fn validate_catalog() {
    let specs = rera_catalog();
    assert_eq!(
        specs.len(),
        specs
            .iter()
            .map(|spec| spec.id)
            .collect::<BTreeSet<_>>()
            .len(),
        "duplicate reraconfig setting id"
    );
    assert_eq!(
        specs.len(),
        specs
            .iter()
            .map(|spec| spec.path)
            .collect::<BTreeSet<_>>()
            .len(),
        "duplicate reraconfig setting path"
    );
    assert_eq!(
        specs.len(),
        specs
            .iter()
            .map(|spec| spec.code)
            .collect::<BTreeSet<_>>()
            .len(),
        "duplicate reraconfig legacy code"
    );
}

pub(super) fn parse_toml_value(
    spec: &ReraConfigSpec,
    item: &Item,
) -> Result<ConfigValue, ReraConfigError> {
    let span = item.span().map(ByteSpan::from);
    let value = item.as_value().ok_or_else(|| {
        error_at(
            ReraConfigErrorKind::InvalidType,
            Some(spec.path),
            span,
            "设置值不是 TOML 标量或数组",
        )
    })?;
    let parsed = match &spec.default {
        ConfigValue::Boolean(_) => value.as_bool().map(ConfigValue::Boolean),
        ConfigValue::Integer(_) => value.as_integer().map(ConfigValue::Integer),
        ConfigValue::String(_) => value
            .as_str()
            .map(|value| ConfigValue::String(value.into())),
        ConfigValue::Enum { allowed, .. } => value.as_str().and_then(|value| {
            enum_from_toml(spec.code, value, allowed).map(|value| ConfigValue::Enum {
                value,
                allowed: allowed.clone(),
            })
        }),
        ConfigValue::Color(_) => parse_color(value).map(ConfigValue::Color),
        ConfigValue::Character(_) => value
            .as_str()
            .and_then(single_character)
            .map(ConfigValue::Character),
        ConfigValue::IntegerList(_) => parse_integer_array(value).map(ConfigValue::IntegerList),
        ConfigValue::StringList(_) => parse_string_array(value).map(ConfigValue::StringList),
    }
    .ok_or_else(|| {
        error_at(
            ReraConfigErrorKind::InvalidValue,
            Some(spec.path),
            span,
            "值的 TOML 类型或内容无效",
        )
    })?;
    validate_config_value(spec, &parsed, span)?;
    Ok(parsed)
}

pub(super) fn validate_config_value(
    spec: &ReraConfigSpec,
    setting: &ConfigValue,
    span: Option<ByteSpan>,
) -> Result<(), ReraConfigError> {
    match (&spec.default, setting) {
        (ConfigValue::Boolean(_), ConfigValue::Boolean(_))
        | (ConfigValue::String(_), ConfigValue::String(_))
        | (ConfigValue::Character(_), ConfigValue::Character(_))
        | (ConfigValue::IntegerList(_), ConfigValue::IntegerList(_))
        | (ConfigValue::StringList(_), ConfigValue::StringList(_)) => {}
        (ConfigValue::Integer(_), ConfigValue::Integer(value)) => {
            if spec.minimum.is_some_and(|minimum| *value < minimum)
                || spec.maximum.is_some_and(|maximum| *value > maximum)
            {
                return Err(error_at(
                    ReraConfigErrorKind::OutOfRange,
                    Some(spec.path),
                    span,
                    "整数超出允许范围",
                ));
            }
        }
        (ConfigValue::Color(_), ConfigValue::Color(value)) if *value <= 0x00ff_ffff => {}
        (ConfigValue::Color(_), ConfigValue::Color(_)) => {
            return Err(error_at(
                ReraConfigErrorKind::OutOfRange,
                Some(spec.path),
                span,
                "RGB 颜色超出 0x000000..=0xffffff",
            ));
        }
        (
            ConfigValue::Enum { allowed, .. },
            ConfigValue::Enum {
                value,
                allowed: actual,
            },
        ) if actual == allowed && allowed.contains(value) => {}
        _ => {
            return Err(error_at(
                ReraConfigErrorKind::InvalidType,
                Some(spec.path),
                span,
                "值类型与设置目录不一致",
            ));
        }
    }
    Ok(())
}

pub(super) fn config_to_toml(spec: &ReraConfigSpec, setting: &ConfigValue) -> Value {
    match setting {
        ConfigValue::Boolean(value) => Value::from(*value),
        ConfigValue::Integer(value) => Value::from(*value),
        ConfigValue::String(value) => Value::from(value.as_str()),
        ConfigValue::Enum { value, .. } => Value::from(enum_to_toml(spec.code, value)),
        ConfigValue::Color(value) => {
            let mut array = Array::new();
            array.push(i64::from((value >> 16) & 0xff));
            array.push(i64::from((value >> 8) & 0xff));
            array.push(i64::from(value & 0xff));
            Value::Array(array)
        }
        ConfigValue::Character(value) => Value::from(value.to_string()),
        ConfigValue::IntegerList(values) => {
            let mut array = Array::new();
            for value in values {
                array.push(*value);
            }
            Value::Array(array)
        }
        ConfigValue::StringList(values) => {
            let mut array = Array::new();
            for value in values {
                array.push(value.as_str());
            }
            Value::Array(array)
        }
    }
}

fn parse_color(value: &Value) -> Option<u32> {
    let values = value.as_array()?;
    if values.len() != 3 {
        return None;
    }
    let parts = values
        .iter()
        .map(Value::as_integer)
        .collect::<Option<Vec<_>>>()?;
    let [red, green, blue] = parts.as_slice() else {
        return None;
    };
    if !parts.iter().all(|value| (0..=255).contains(value)) {
        return None;
    }
    Some(
        (u32::try_from(*red).ok()? << 16)
            | (u32::try_from(*green).ok()? << 8)
            | u32::try_from(*blue).ok()?,
    )
}

fn parse_integer_array(value: &Value) -> Option<Vec<i64>> {
    value.as_array()?.iter().map(Value::as_integer).collect()
}

fn parse_string_array(value: &Value) -> Option<Vec<String>> {
    value
        .as_array()?
        .iter()
        .map(|value| value.as_str().map(Into::into))
        .collect()
}

fn single_character(value: &str) -> Option<char> {
    let mut characters = value.chars();
    let character = characters.next()?;
    characters.next().is_none().then_some(character)
}

pub(super) fn enum_to_toml(code: &str, value: &str) -> String {
    match (code, value) {
        ("useLanguage", "CHINESE_HANS") => "simplified_chinese".into(),
        ("useLanguage", "CHINESE_HANT") => "traditional_chinese".into(),
        _ => value.to_ascii_lowercase(),
    }
}

pub(super) fn enum_from_toml(code: &str, value: &str, allowed: &[String]) -> Option<String> {
    allowed
        .iter()
        .find(|candidate| enum_to_toml(code, candidate) == value)
        .cloned()
}

fn integer_bounds(code: &str, default: &ConfigValue) -> (Option<i64>, Option<i64>) {
    if !matches!(default, ConfigValue::Integer(_)) {
        return (None, None);
    }
    let i32_bounds = (Some(i64::from(i32::MIN)), Some(i64::from(i32::MAX)));
    match code {
        "AudioVolume" => (Some(0), Some(100)),
        "WindowX" | "WindowY" => (Some(128), Some(i64::from(i32::MAX))),
        "MaxLog" => (Some(500), Some(i64::from(i32::MAX))),
        "PrintCPerLine" | "PrintCLength" | "ScrollHeight" => (Some(1), Some(i64::from(i32::MAX))),
        "FontSize" | "LineHeight" => (Some(8), Some(i64::from(i32::MAX))),
        "DisplayWarningLevel" => (Some(0), Some(255)),
        "SaveDataNos" => (Some(20), Some(80)),
        "pbandDef" | "RelationDef" => (Some(i64::MIN), Some(i64::MAX)),
        _ => i32_bounds,
    }
}

#[allow(clippy::too_many_lines)]
fn metadata(code: &str) -> (u16, &'static str, &'static str, bool) {
    match code {
        "IgnoreCase" => (1, "script.ignore_case", "名称比较时忽略大小写差异", false),
        "UseRenameFile" => (
            2,
            "project.use_rename_file",
            "加载并应用 _Rename.csv",
            false,
        ),
        "UseReplaceFile" => (
            3,
            "replacement.enabled",
            "启用 replacement 表中的替换设置",
            false,
        ),
        "UseMouse" => (4, "input.mouse_enabled", "允许鼠标点击游戏交互项", false),
        "UseMenu" => (
            5,
            "interface.menu_mode",
            "控制客户端菜单始终显示、按页面高度自动显示或始终隐藏",
            false,
        ),
        "UseDebugCommand" => (6, "debug.commands_enabled", "允许执行调试命令", false),
        "AllowMultipleInstances" => (
            7,
            "application.allow_multiple_instances",
            "允许同时启动多个实例",
            false,
        ),
        "AutoSave" => (8, "save.auto_save", "商店事件结束后执行自动保存", false),
        "UseKeyMacro" => (9, "input.keyboard_macros_enabled", "启用键盘宏", false),
        "SizableWindow" => (10, "window.resizable", "允许调整窗口大小", false),
        "WindowX" => (12, "window.width", "设置游戏主视口宽度", false),
        "WindowY" => (13, "window.height", "设置游戏主视口高度", false),
        "WindowPosX" => (14, "window.position_x", "设置原生窗口起始横坐标", false),
        "WindowPosY" => (15, "window.position_y", "设置原生窗口起始纵坐标", false),
        "SetWindowPos" => (
            16,
            "window.use_saved_position",
            "启动时应用保存的窗口位置",
            false,
        ),
        "WindowMaximixed" => (17, "window.start_maximized", "启动时最大化原生窗口", false),
        "MaxLog" => (18, "output.history_lines", "限制物理历史日志行数", false),
        "PrintCPerLine" => (
            19,
            "output.printc_items_per_line",
            "设置每行 PRINTC 项数",
            false,
        ),
        "PrintCLength" => (
            20,
            "output.printc_item_width",
            "设置 PRINTC 项的目标字符宽度",
            false,
        ),
        "FontName" => (21, "text.font_family", "设置游戏默认字体族", false),
        "FontSize" => (22, "text.font_size", "设置游戏默认字号", false),
        "LineHeight" => (23, "text.line_height", "设置游戏默认行高", false),
        "ForeColor" => (24, "color.text", "设置默认文字颜色", false),
        "BackColor" => (25, "color.background", "设置默认背景颜色", false),
        "FocusColor" => (26, "color.selection", "设置选中项文字颜色", false),
        "LogColor" => (27, "color.history", "设置历史日志文字颜色", false),
        "FPS" => (28, "display.frames_per_second", "设置目标刷新帧率", false),
        "ScrollHeight" => (30, "output.scroll_lines", "设置每次滚动的行数", false),
        "InfiniteLoopAlertTime" => (
            31,
            "runtime.infinite_loop_warning_ms",
            "设置无限循环警告等待时间",
            false,
        ),
        "DisplayWarningLevel" => (
            32,
            "diagnostics.minimum_warning_level",
            "过滤低于阈值的加载诊断",
            false,
        ),
        "StrictUserCallArguments" => (
            128,
            "diagnostics.strict_user_call_arguments",
            "将蛇版用户函数多余实参警告提升为错误；不改变内置函数参数规则",
            false,
        ),
        "DisableBeforeErrorThrow" => (
            129,
            "runtime.disable_before_error_throw",
            "禁用蛇版 BEFORE_ERROR 与 BEFORE_THROW 最终故障钩子",
            false,
        ),
        "IgnoreUncalledFunction" => (
            35,
            "diagnostics.ignore_uncalled_functions",
            "忽略未调用函数的处理和警告",
            false,
        ),
        "FunctionNotFoundWarning" => (
            36,
            "diagnostics.missing_function",
            "设置找不到函数时的警告策略",
            false,
        ),
        "FunctionNotCalledWarning" => (
            37,
            "diagnostics.uncalled_function",
            "设置函数未调用时的警告策略",
            false,
        ),
        "ChangeMasterNameIfDebug" => (
            38,
            "debug.rename_master",
            "执行调试命令时修改 MASTER 名称",
            false,
        ),
        "ButtonWrap" => (
            39,
            "output.keep_buttons_on_one_line",
            "避免在按钮内容中途换行",
            false,
        ),
        "SearchSubdirectory" => (
            40,
            "project.search_subdirectories",
            "递归搜索项目子目录",
            false,
        ),
        "SortWithFilename" => (
            41,
            "project.sort_files_by_name",
            "按文件名稳定排序加载顺序",
            false,
        ),
        "SaveDataNos" => (43, "save.slots_per_page", "设置存档菜单每页槽位数", false),
        "WarnBackCompatibility" => (
            44,
            "diagnostics.eramaker_compatibility",
            "显示 eramaker 兼容性警告",
            false,
        ),
        "AllowFunctionOverloading" => (
            45,
            "script.allow_system_function_override",
            "允许覆盖系统函数",
            false,
        ),
        "WarnFunctionOverloading" => (
            46,
            "diagnostics.system_function_override",
            "系统函数被覆盖时显示警告",
            false,
        ),
        "WarnNormalFunctionOverloading" => (
            50,
            "diagnostics.duplicate_normal_functions",
            "普通函数重名时显示警告",
            false,
        ),
        "CompatiErrorLine" => (
            51,
            "project.continue_with_parse_errors",
            "存在无法解析的行时仍允许启动",
            false,
        ),
        "CompatiCALLNAME" => (
            52,
            "compatibility.callname_falls_back_to_name",
            "CALLNAME 为空时回退到 NAME",
            false,
        ),
        "CompatiRAND" => (
            54,
            "compatibility.eramaker_rand",
            "使用 eramaker RAND 兼容语义",
            false,
        ),
        "SystemAllowFullSpace" => (
            57,
            "script.full_width_space_is_whitespace",
            "把全角空格视为脚本空白",
            false,
        ),
        "CompatiLinefeedAs1739" => (
            58,
            "compatibility.legacy_nonbutton_wrapping",
            "重现 1739 版以前的非按钮换行",
            false,
        ),
        "useLanguage" => (
            59,
            "compatibility.east_asian_language",
            "选择传统东亚编码语义",
            false,
        ),
        "AllowLongInputByMouse" => (
            60,
            "input.allow_long_oneinput_from_mouse",
            "允许鼠标向 ONEINPUT 提交多个字符",
            false,
        ),
        "CompatiCallEvent" => (
            61,
            "script.allow_calling_event_functions",
            "允许普通 CALL 调用事件函数",
            false,
        ),
        "CompatiSPChara" => (
            62,
            "compatibility.sp_characters",
            "启用 SP 角色兼容行为",
            false,
        ),
        "SystemSaveInBinary" => (63, "save.binary_format", "使用传统二进制存档格式", false),
        "CompatiFuncArgOptional" => (
            64,
            "script.allow_omitted_function_arguments",
            "允许省略用户函数的全部参数",
            false,
        ),
        "CompatiFuncArgAutoConvert" => (
            65,
            "script.auto_convert_function_arguments_to_string",
            "为字符串参数自动补充 TOSTR",
            false,
        ),
        "SystemIgnoreTripleSymbol" => (
            66,
            "script.preserve_triple_symbols_in_form",
            "不展开 FORM 中的三连符号",
            false,
        ),
        "TimesNotRigorousCalculation" => (
            67,
            "compatibility.eramaker_times",
            "使用 eramaker TIMES 计算语义",
            false,
        ),
        "SystemNoTarget" => (
            68,
            "compatibility.require_character_variable_arguments",
            "要求显式提供角色变量参数",
            false,
        ),
        "SystemIgnoreStringSet" => (
            69,
            "compatibility.require_string_expression_assignment",
            "字符串变量赋值必须使用字符串表达式",
            false,
        ),
        "ForbidUpdateCheck" => (
            70,
            "network.disable_update_check",
            "禁止脚本执行联网更新检查",
            false,
        ),
        "UseERD" => (71, "project.erd_enabled", "加载并应用 ERD 扩展定义", false),
        "VarsizeDimConfig" => (
            72,
            "compatibility.erd_varsize_dimensions",
            "VARSIZE 使用 ERD 的一基维度编号",
            false,
        ),
        "CheckDuplicateIdentifier" => (
            73,
            "diagnostics.erd_local_identifier_conflict",
            "检查 ERD 标识符与局部变量重名",
            false,
        ),
        "ReplaceContinuationBR" => (
            74,
            "script.continuation_newline_replacement",
            "设置续行物理换行的替换文本",
            false,
        ),
        "PluginAvailableWarn" => (
            75,
            "diagnostics.external_plugin_enabled",
            "外部插件启用时显示警告",
            false,
        ),
        "ValidExtension" => (
            76,
            "file_io.text_file_extensions",
            "限制 LOADTEXT 和 SAVETEXT 扩展名",
            false,
        ),
        "ZipSaveData" => (77, "save.compress_binary", "压缩传统二进制存档", false),
        "EmueraIcon" => (80, "window.icon_path", "设置原生窗口图标路径", false),
        "Ctrl_Z_Enabled" => (100, "input.undo_enabled", "启用 Ctrl+Z 输入撤销", false),
        "MoneyLabel" => (108, "replacement.currency_label", "设置货币单位文本", false),
        "MoneyFirst" => (
            109,
            "replacement.currency_before_amount",
            "把货币单位显示在金额之前",
            false,
        ),
        "LoadLabel" => (
            110,
            "replacement.loading_message",
            "设置加载提示文本",
            false,
        ),
        "MaxShopItem" => (
            111,
            "replacement.maximum_shop_items",
            "设置商店物品容量",
            false,
        ),
        "DrawLineString" => (
            112,
            "replacement.drawline_text",
            "设置 DRAWLINE 使用的文本",
            false,
        ),
        "BarChar1" => (
            113,
            "replacement.bar_filled_character",
            "设置 BAR 已填充字符",
            false,
        ),
        "BarChar2" => (
            114,
            "replacement.bar_empty_character",
            "设置 BAR 未填充字符",
            false,
        ),
        "TitleMenuString0" => (
            115,
            "replacement.new_game_label",
            "设置新游戏菜单文本",
            false,
        ),
        "TitleMenuString1" => (
            116,
            "replacement.load_game_label",
            "设置读取存档菜单文本",
            false,
        ),
        "ComAbleDefault" => (
            117,
            "replacement.default_com_able",
            "设置 COM_ABLE 默认值",
            false,
        ),
        "StainDefault" => (
            118,
            "replacement.default_stain",
            "设置 STAIN 等级默认数组",
            false,
        ),
        "TimeupLabel" => (
            119,
            "replacement.time_up_message",
            "设置时间耗尽提示文本",
            false,
        ),
        "ExpLvDef" => (
            120,
            "replacement.default_experience_levels",
            "设置 EXPLV 阈值数组",
            false,
        ),
        "PalamLvDef" => (
            121,
            "replacement.default_palam_levels",
            "设置 PALAMLV 阈值数组",
            false,
        ),
        "pbandDef" => (122, "replacement.default_pband", "设置 PBAND 默认值", false),
        "RelationDef" => (
            123,
            "replacement.default_relation",
            "设置 RELATION 默认值",
            false,
        ),
        "AudioVolume" => (125, "audio.volume", "设置游戏主音量百分比", false),
        "ReplaceFullWidthSpaces" => (
            126,
            "text.replace_full_width_spaces",
            "把游戏输出中的全角空格显示为两个半角空格",
            false,
        ),
        "CharacterWidthMode" => (
            127,
            "text.character_width_mode",
            "选择运行时格式化与客户端显示共用的字符列宽计算策略；automatic 在 CJK 兼容宽度基础上把无文本变体标记的 Unicode 图形符号按宽字符处理，ambiguous_narrow 和 ambiguous_wide 分别把 East Asian Ambiguous 字符按窄字符和宽字符处理",
            false,
        ),
        _ => panic!("missing reraconfig metadata for {code}"),
    }
}
