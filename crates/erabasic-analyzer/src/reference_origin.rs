//! Source token semantics for the fixed reference. These are not VM permissions.
use erabasic_data::VariableSchema;
use erabasic_hir::ReferenceVariableSemantics;

pub(crate) fn variable_semantics(
    schema: &VariableSchema,
    reference: bool,
    declared: bool,
) -> ReferenceVariableSemantics {
    if reference {
        return ReferenceVariableSemantics {
            is_const: false,
            can_restructure: false,
        };
    }
    if declared {
        // Function-private CONST declarations use Local storage even though
        // their source token still has CONST restructure semantics. Within
        // this declared-only branch, immutable means the CONST modifier; the
        // readonly pseudo-variable exception below does not apply.
        let is_const = !schema.mutable;
        return ReferenceVariableSemantics {
            is_const,
            can_restructure: is_const,
        };
    }
    // Creator/VariableData creates these exact tokens. Never infer either flag
    // from !schema.mutable: readonly pseudo variables can change during execution.
    let constant_token = matches!(
        schema.id.name().to_ascii_uppercase().as_str(),
        "ITEMPRICE"
            | "ABLNAME"
            | "TALENTNAME"
            | "EXPNAME"
            | "MARKNAME"
            | "PALAMNAME"
            | "ITEMNAME"
            | "TRAINNAME"
            | "BASENAME"
            | "SOURCENAME"
            | "EXNAME"
            | "EQUIPNAME"
            | "TEQUIPNAME"
            | "FLAGNAME"
            | "TFLAGNAME"
            | "CFLAGNAME"
            | "TCVARNAME"
            | "CSTRNAME"
            | "STAINNAME"
            | "CDFLAGNAME1"
            | "CDFLAGNAME2"
            | "STRNAME"
            | "TSTRNAME"
            | "SAVESTRNAME"
            | "GLOBALNAME"
            | "GLOBALSNAME"
            | "DAYNAME"
            | "TIMENAME"
            | "MONEYNAME"
            | "GAMEBASE_AUTHOR"
            | "GAMEBASE_AUTHER"
            | "GAMEBASE_INFO"
            | "GAMEBASE_YEAR"
            | "GAMEBASE_TITLE"
            | "GAMEBASE_URL"
            | "GAMEBASE_VERSIONNAME"
            | "GAMEBASE_GAMECODE"
            | "GAMEBASE_VERSION"
            | "GAMEBASE_ALLOWVERSION"
            | "GAMEBASE_DEFAULTCHARA"
            | "GAMEBASE_NOITEM"
            | "MONEYLABEL"
            | "DRAWLINESTR"
            | "__FILE__"
            | "__FUNCTION__"
            | "__LINE__"
            | "__INT_MAX__"
            | "__INT_MIN__"
            | "EMUERA_VERSION"
    );
    let readonly_pseudo = matches!(
        schema.id.name().to_ascii_uppercase().as_str(),
        "RAND"
            | "CHARANUM"
            | "LASTLOAD_TEXT"
            | "LASTLOAD_VERSION"
            | "LASTLOAD_NO"
            | "LINECOUNT"
            | "ISTIMEOUT"
    );
    ReferenceVariableSemantics {
        is_const: constant_token || readonly_pseudo,
        can_restructure: constant_token,
    }
}
