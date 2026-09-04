use erabasic_hir::SemanticType::{Integer as IntType, String as StrType};

use super::{
    ArgumentConstraint::{Integer, MutableInteger, ReferenceAny, String},
    FunctionCatalog,
};

pub(super) fn register(catalog: &mut FunctionCatalog<'_>) {
    register_display_queries(catalog);
    register_canvas_resources(catalog);
    register_canvas_drawing(catalog);
    register_snake_display(catalog);
}

fn register_display_queries(catalog: &mut FunctionCatalog<'_>) {
    catalog.add("CURRENTALIGN", StrType, &[], 0, false);
    catalog.add("CURRENTREDRAW", IntType, &[], 0, false);
    catalog.add("GETFONT", StrType, &[], 0, false);
    catalog.add("GETSTYLE", IntType, &[], 0, false);
    catalog.add("GGETCOLOR", IntType, &[Integer, Integer, Integer], 3, false);
    catalog.add(
        "GGETTEXTSIZE",
        IntType,
        &[String, String, Integer, Integer],
        3,
        false,
    );
    for name in [
        "GETBGCOLOR",
        "GETCOLOR",
        "GETDEFBGCOLOR",
        "GETDEFCOLOR",
        "GETFOCUSCOLOR",
    ] {
        catalog.add(name, IntType, &[], 0, false);
    }
    for name in ["HTML_ESCAPE", "HTML_TOPLAINTEXT"] {
        catalog.add(name, StrType, &[String], 1, false);
    }
}

fn register_canvas_resources(catalog: &mut FunctionCatalog<'_>) {
    catalog.add("GCREATE", IntType, &[Integer, Integer, Integer], 3, false);
    catalog.add(
        "GCREATEFROMFILE",
        IntType,
        &[Integer, String, Integer],
        2,
        false,
    );
    for name in ["GLOAD", "GSAVE", "GSETBRUSH"] {
        catalog.add(name, IntType, &[Integer, Integer], 2, false);
    }
    for name in [
        "GCREATED",
        "GDISPOSE",
        "GWIDTH",
        "GHEIGHT",
        "GGETBRUSH",
        "GGETPEN",
        "GGETPENWIDTH",
        "GGETFONTSIZE",
        "GGETFONTSTYLE",
    ] {
        catalog.add(name, IntType, &[Integer], 1, false);
    }
    catalog.add("GGETFONT", StrType, &[Integer], 1, false);
    catalog.add(
        "GSETCOLOR",
        IntType,
        &[Integer, Integer, Integer, Integer],
        4,
        false,
    );
    catalog.add("GSETPEN", IntType, &[Integer, Integer, Integer], 3, false);
    catalog.add(
        "GDASHSTYLE",
        IntType,
        &[Integer, Integer, Integer],
        3,
        false,
    );
    catalog.add(
        "GSETFONT",
        IntType,
        &[Integer, String, Integer, Integer],
        3,
        false,
    );
}

fn register_canvas_drawing(catalog: &mut FunctionCatalog<'_>) {
    catalog.add("GFILLRECTANGLE", IntType, &[Integer; 5], 5, false);
    catalog.add("GDRAWLINE", IntType, &[Integer; 5], 5, false);
    catalog.add(
        "GDRAWTEXT",
        IntType,
        &[Integer, String, Integer, Integer],
        2,
        false,
    );
    catalog.add("GGETCOLOR", IntType, &[Integer, Integer, Integer], 3, false);
    catalog.add("GDRAWGWITHMASK", IntType, &[Integer; 5], 5, false);
    catalog.add("GDRAWGWITHROTATE", IntType, &[Integer; 5], 3, false);
    catalog.add(
        "GDRAWG",
        IntType,
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
        10,
        false,
    );
    catalog.add(
        "GDRAWSPRITE",
        IntType,
        &[
            Integer,
            String,
            Integer,
            Integer,
            Integer,
            Integer,
            ReferenceAny,
        ],
        2,
        false,
    );
    catalog.add(
        "SPRITECREATE",
        IntType,
        &[
            String, Integer, Integer, Integer, Integer, Integer, Integer, Integer, Integer, Integer,
        ],
        2,
        false,
    );
    catalog.add(
        "SPRITECREATEFROMFILE",
        IntType,
        &[String, String, Integer],
        2,
        false,
    );
}

fn register_snake_display(catalog: &mut FunctionCatalog<'_>) {
    for name in ["G_POLYGON_DRAW", "G_POLYGON_FILL", "G_POLYGON_POINT_CLEAR"] {
        catalog.add(name, IntType, &[Integer], 1, false);
    }
    catalog.add(
        "G_POLYGON_POINT_ADD",
        IntType,
        &[Integer, Integer, Integer],
        3,
        false,
    );
    catalog.add("MOVETEXTBOX", IntType, &[Integer; 3], 3, false);
    catalog.add("RESUMETEXTBOX", IntType, &[Integer; 3], 3, false);
    catalog.add("BITMAP_CACHE_ENABLE", IntType, &[Integer], 1, false);
    catalog.add("SETANIMETIMER", IntType, &[Integer], 1, false);
    catalog.add("GETANIMETIMER", IntType, &[], 0, false);
    for name in ["CBGCLEAR", "CBGCLEARBUTTON", "CBGREMOVEBMAP"] {
        catalog.add(name, IntType, &[], 0, false);
    }
    catalog.add("CBGREMOVERANGE", IntType, &[Integer, Integer], 2, false);
    catalog.add(
        "CBGSETG",
        IntType,
        &[Integer, Integer, Integer, Integer],
        4,
        false,
    );
    catalog.add("CBGSETBMAPG", IntType, &[Integer], 1, false);
    catalog.add(
        "CBGSETSPRITE",
        IntType,
        &[
            String,
            Integer,
            Integer,
            Integer,
            Integer,
            Integer,
            Integer,
            ReferenceAny,
        ],
        1,
        false,
    );
    catalog.add(
        "CBGSETBUTTONSPRITE",
        IntType,
        &[Integer, String, String, Integer, Integer, Integer, String],
        6,
        false,
    );
    catalog.add("EXISTSIMAGELAYER", IntType, &[Integer], 1, false);
    catalog.add("GETLINEY", IntType, &[Integer], 1, false);
    catalog.add(
        "BITSET",
        IntType,
        &[MutableInteger, Integer, Integer, Integer],
        2,
        false,
    );
    for name in ["BITGET", "BITTOGGLE", "BITINDEXOFFIRST"] {
        catalog.add(name, IntType, &[MutableInteger, Integer], 1, false);
    }
}
