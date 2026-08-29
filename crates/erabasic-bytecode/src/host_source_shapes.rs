//! Fixed source function shapes only. This table does not grant Host execution.
//! Pinned snake fc4fb214 / original 26a35dc `Creator.Method` + `FunctionMethod`.
//! `ArgTypeList` with `OmitStart=-1` permits explicit null slots; `EraType[]` does not.
use crate::{RuntimeArgumentConstraint, RuntimeCallableShape};
use RuntimeArgumentConstraint::{Any, Integer, MutableString, ReferenceAny, String};

#[must_use]
pub fn canonical_host_source_shapes(name: &str, snake: bool) -> Option<Vec<RuntimeCallableShape>> {
    let name = name.to_ascii_lowercase();
    profile_shapes(&name, snake)
        .or_else(|| common_shapes(&name))
        .or_else(|| graphics_shapes(&name))
        .or_else(|| service_shapes(&name))
        .or_else(|| sql_shapes(&name, snake))
}

fn profile_shapes(name: &str, snake: bool) -> Option<Vec<RuntimeCallableShape>> {
    Some(if snake {
        match name {
            "getanimetimer" | "text_bgc_off" => vec![shape(0, 0, 0, false, &[])],
            "setanimetimer" | "bitmap_cache_enable" => {
                vec![shape(1, 1, 1, false, &[Integer])]
            }
            "text_bgc_on" => vec![shape(2, 2, 2, false, &[Integer, Integer])],
            "cbgsetsprite" => vec![shape(
                1,
                8,
                1,
                false,
                &[
                    String, Integer, Integer, Integer, Integer, Integer, Integer, Any,
                ],
            )],
            "spritecreate" => vec![
                shape(2, 2, 0, true, &[String, Integer]),
                shape(
                    6,
                    6,
                    0,
                    true,
                    &[String, Integer, Integer, Integer, Integer, Integer],
                ),
                shape(
                    8,
                    8,
                    0,
                    true,
                    &[
                        String, Integer, Integer, Integer, Integer, Integer, Integer, Integer,
                    ],
                ),
                shape(
                    10,
                    10,
                    0,
                    true,
                    &[
                        String, Integer, Integer, Integer, Integer, Integer, Integer, Integer,
                        Integer, Integer,
                    ],
                ),
            ],
            _ => return None,
        }
    } else {
        match name {
            "bitmap_cache_enable" | "setanimetimer" => vec![shape(1, 1, 1, false, &[Integer])],
            "cbgsetsprite" => vec![shape(4, 4, 4, false, &[String, Integer, Integer, Integer])],
            "spritecreate" => vec![
                shape(2, 2, 0, true, &[String, Integer]),
                shape(
                    6,
                    6,
                    0,
                    true,
                    &[String, Integer, Integer, Integer, Integer, Integer],
                ),
            ],
            _ => return None,
        }
    })
}

fn common_shapes(name: &str) -> Option<Vec<RuntimeCallableShape>> {
    Some(match name {
        "barstr" | "gcreate" | "gdashstyle" | "ggetcolor" | "gsetpen" | "movetextbox"
        | "resumetextbox" => vec![shape(3, 3, 3, false, &[Integer, Integer, Integer])],
        "cbgclear"
        | "cbgclearbutton"
        | "cbgremovebmap"
        | "clientheight"
        | "clientwidth"
        | "currentalign"
        | "currentredraw"
        | "getbgcolor"
        | "getcolor"
        | "getdefbgcolor"
        | "getdefcolor"
        | "getdoingfunction"
        | "getfocuscolor"
        | "getfont"
        | "getmemoryusage"
        | "getmillisecond"
        | "getsecond"
        | "getstyle"
        | "gettextbox"
        | "gettime"
        | "gettimes"
        | "html_popprintingstr"
        | "isactive"
        | "isskip"
        | "lineisempty"
        | "messkip"
        | "mouseb"
        | "mouseskip"
        | "mousex"
        | "mousey"
        | "printclength"
        | "printcperline"
        | "savenos" => vec![shape(0, 0, 0, false, &[])],
        "cbgremoverange" | "gload" | "gsave" | "gsetbrush" => {
            vec![shape(2, 2, 2, false, &[Integer, Integer])]
        }
        "cbgsetbmapg" | "chkdata" | "gcreated" | "gdispose" | "getkey" | "getkeytriggered"
        | "ggetbrush" | "ggetfont" | "ggetfontsize" | "ggetfontstyle" | "ggetpen"
        | "ggetpenwidth" | "gheight" | "gwidth" | "hotkey_state_init" | "spritedisposeall" => {
            vec![shape(1, 1, 1, false, &[Integer])]
        }
        "cbgsetbuttonsprite" => vec![shape(
            6,
            7,
            6,
            false,
            &[Integer, String, String, Integer, Integer, Integer, String],
        )],
        "cbgsetg" | "gsetcolor" => {
            vec![shape(4, 4, 4, false, &[Integer, Integer, Integer, Integer])]
        }
        "chkcharadata" | "chkfont" | "existfile" | "existsound" | "getconfig" | "getconfigs"
        | "getlinestr" | "html_escape" | "html_toplaintext" | "settextbox" | "spritecreated"
        | "spritedispose" | "spriteheight" | "spriteposx" | "spriteposy" | "spritewidth"
        | "tofull" | "tohalf" => vec![shape(1, 1, 1, false, &[String])],
        "enumfiles" => vec![shape(
            1,
            4,
            1,
            false,
            &[String, String, Integer, MutableString],
        )],
        "enumfuncbeginswith" | "enumfuncendswith" | "enumfuncwith" | "enumvarbeginswith"
        | "enumvarendswith" | "enumvarwith" => {
            vec![shape(1, 2, 1, false, &[String, MutableString])]
        }
        "existfunction" | "html_stringlen" | "varsize" => {
            vec![shape(1, 2, 1, false, &[String, Integer])]
        }
        "find_charadata" => vec![shape(0, 1, 0, false, &[String])],
        "flowinput" => vec![shape(1, 4, 1, false, &[Integer, Integer, Integer, Integer])],
        "flowinputs" | "moneystr" | "tostr" => vec![shape(1, 2, 1, false, &[Integer, String])],
        _ => return None,
    })
}

fn graphics_shapes(name: &str) -> Option<Vec<RuntimeCallableShape>> {
    Some(match name {
        "gclear" => vec![
            shape(2, 2, 0, true, &[Integer, Integer]),
            shape(
                6,
                6,
                0,
                true,
                &[Integer, Integer, Integer, Integer, Integer, Integer],
            ),
        ],
        "gcreatefromfile" => vec![shape(2, 3, 2, false, &[Integer, String, Integer])],
        "gdrawg" => vec![shape(
            10,
            11,
            10,
            false,
            &[
                Integer,
                Integer,
                Integer,
                Integer,
                Integer,
                Integer,
                Integer,
                Integer,
                Integer,
                Integer,
                ReferenceAny,
            ],
        )],
        "gdrawgwithmask" | "gdrawline" | "gfillrectangle" => vec![shape(
            5,
            5,
            5,
            false,
            &[Integer, Integer, Integer, Integer, Integer],
        )],
        "gdrawgwithrotate" => vec![shape(
            3,
            5,
            3,
            false,
            &[Integer, Integer, Integer, Integer, Integer],
        )],
        "gdrawsprite" => vec![
            shape(2, 2, 0, true, &[Integer, String]),
            shape(4, 4, 0, true, &[Integer, String, Integer, Integer]),
            shape(
                6,
                7,
                6,
                false,
                &[
                    Integer,
                    String,
                    Integer,
                    Integer,
                    Integer,
                    Integer,
                    ReferenceAny,
                ],
            ),
        ],
        "gdrawtext" | "gsetfont" => {
            vec![shape(2, 4, 2, false, &[Integer, String, Integer, Integer])]
        }
        "getdisplayline" => vec![shape(1, 1, 0, true, &[Integer])],
        "ggettextsize" => vec![shape(3, 4, 3, false, &[String, String, Integer, Integer])],
        _ => return None,
    })
}

fn service_shapes(name: &str) -> Option<Vec<RuntimeCallableShape>> {
    Some(match name {
        "hotkey_state" => vec![shape(1, 2, 1, false, &[Integer, Integer])],
        "html_getprintedstr" => vec![shape(0, 1, 0, false, &[Integer])],
        "html_stringlines" | "html_substring" => vec![shape(2, 2, 2, false, &[String, Integer])],
        "loadtext" => vec![shape(1, 3, 1, false, &[Any, Integer, Integer])],
        "outputlog" => vec![shape(0, 2, 0, false, &[String, Integer])],
        "savetext" => vec![shape(2, 4, 2, false, &[String, Any, Integer, Integer])],
        "spriteanimeaddframe" => vec![shape(
            9,
            9,
            9,
            false,
            &[
                String, Integer, Integer, Integer, Integer, Integer, Integer, Integer, Integer,
            ],
        )],
        "spriteanimecreate" | "spritegetcolor" | "spritemove" | "spritesetpos" => {
            vec![shape(3, 3, 3, false, &[String, Integer, Integer])]
        }
        _ => return None,
    })
}

fn sql_shapes(name: &str, snake: bool) -> Option<Vec<RuntimeCallableShape>> {
    if !snake {
        return None;
    }
    let exact = |arguments: &[RuntimeArgumentConstraint]| {
        vec![shape(
            arguments.len(),
            arguments.len(),
            arguments.len(),
            false,
            arguments,
        )]
    };
    Some(match name {
        "sql_connect" => vec![RuntimeCallableShape {
            minimum: 1,
            maximum: Some(2),
            omitted_from: 1,
            arguments: vec![String, String],
            allow_omitted: true,
        }],
        "sql_disconnect" => exact(&[String]),
        "sql_execute_nonquery"
        | "sql_execute_reader"
        | "sql_execute_scalar_long"
        | "sql_execute_scalar_string" => exact(&[String, String]),
        "sql_reader_read" | "sql_reader_close" => exact(&[Integer]),
        "sql_reader_get_long" | "sql_reader_get_string" | "sql_reader_isnull" => {
            exact(&[Integer, Integer])
        }
        "sql_import_map_xml" => exact(&[String, String, String]),
        "sql_p_execute_nonquery"
        | "sql_p_execute_reader"
        | "sql_p_execute_scalar_long"
        | "sql_p_execute_scalar_string" => vec![RuntimeCallableShape {
            minimum: 2,
            maximum: None,
            omitted_from: 2,
            arguments: vec![String, String, String],
            allow_omitted: true,
        }],
        _ => return None,
    })
}

fn shape(
    minimum: usize,
    maximum: usize,
    omitted_from: usize,
    allow_omitted: bool,
    arguments: &[RuntimeArgumentConstraint],
) -> RuntimeCallableShape {
    RuntimeCallableShape {
        minimum,
        maximum: Some(maximum),
        omitted_from,
        arguments: arguments.to_vec(),
        allow_omitted,
    }
}
/// Additional source token ranks, checked from authoritative variable definitions.
#[must_use]
pub fn host_source_place_ranks(
    name: &str,
    slot: usize,
) -> Option<(&'static [usize], crate::BytecodeType)> {
    use crate::BytecodeType::{Integer, String};
    match (name.to_ascii_lowercase().as_str(), slot) {
        ("gdrawg", 10) | ("gdrawsprite", 6) => Some((&[2, 3], Integer)),
        ("enumfiles", 3)
        | (
            "enumfuncbeginswith" | "enumfuncendswith" | "enumfuncwith" | "enumvarbeginswith"
            | "enumvarendswith" | "enumvarwith",
            1,
        ) => Some((&[1], String)),
        _ => None,
    }
}
