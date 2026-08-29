//! Single transaction implementation for bytecode and runtime-form Native calls.
#[allow(clippy::wildcard_imports)]
use super::*;
impl Vm {
    pub(super) fn execute_special_native(
        &mut self,
        fiber: &mut Fiber,
        native_name: &str,
        arguments: &[VmValue],
        omitted_arguments: &[usize],
    ) -> Result<Option<NativeReady>, VmError> {
        let ready = if native_name == "__mutate_integer" {
            NativeReady::value(execute_integer_mutation(self, fiber, arguments)?)
        } else if matches!(native_name, "swap" | "swapvar") {
            execute_swap_transaction(self, fiber, arguments)?;
            NativeReady::default()
        } else if matches!(native_name, "setbit" | "clearbit" | "invertbit") {
            execute_bit_mutation(self, fiber, native_name, arguments)?;
            NativeReady::default()
        } else if native_name == "split" {
            execute_split_transaction(self, fiber, arguments)?;
            NativeReady::default()
        } else if native_name == "getnum" {
            NativeReady::value(execute_getnum(self, fiber, arguments)?)
        } else if native_name == "erdname" {
            NativeReady::value(execute_erdname(self, fiber, arguments)?)
        } else if native_name == "__indexbyname" {
            NativeReady::value(execute_index_by_name(self, fiber, arguments)?)
        } else if native_name == "setvar" {
            NativeReady::value(execute_set_var(self, fiber, arguments)?)
        } else if matches!(native_name, "getvar" | "getvars") {
            NativeReady::value(execute_get_var(
                self,
                fiber,
                arguments,
                native_name == "getvars",
            )?)
        } else if native_name == "__encodetouni_result" {
            execute_encode_to_uni_result(self, fiber, arguments)?;
            NativeReady::default()
        } else if native_name == "strjoin" {
            NativeReady::value(execute_strjoin(self, fiber, arguments, omitted_arguments)?)
        } else if matches!(native_name, "arrayremove" | "arrayshift" | "arraysort") {
            execute_array_mutation(self, fiber, native_name, arguments)?;
            NativeReady::default()
        } else if native_name == "arraycopy" {
            execute_array_copy(self, fiber, arguments)?;
            NativeReady::default()
        } else if matches!(native_name, "varset" | "cvarset") {
            execute_variable_fill(self, fiber, native_name, arguments)?;
            NativeReady::default()
        } else if native_name == "arraymsort" {
            NativeReady::value(execute_array_multi_sort(self, fiber, arguments)?)
        } else if native_name == "arraymsortex" {
            NativeReady::value(execute_array_multi_sort_ex(self, fiber, arguments)?)
        } else if matches!(native_name, "findelement" | "findlastelement") {
            NativeReady::value(execute_find_element(
                self,
                fiber,
                native_name == "findlastelement",
                arguments,
            )?)
        } else if native_name == "regexpmatch" {
            NativeReady::value(execute_regex_match(self, fiber, arguments)?)
        } else if is_array_query(native_name) {
            NativeReady::value(execute_array_query(self, fiber, native_name, arguments)?)
        } else if is_character_query(native_name) {
            NativeReady::value(execute_character_query(
                self,
                fiber,
                native_name,
                arguments,
            )?)
        } else if matches!(
            native_name,
            "addchara"
                | "addspchara"
                | "adddefchara"
                | "addvoidchara"
                | "delchara"
                | "delallchara"
                | "swapchara"
                | "copychara"
                | "addcopychara"
                | "pickupchara"
                | "sortchara"
                | "reset_stain"
        ) {
            execute_character_mutation(self, native_name, arguments)?;
            NativeReady::default()
        } else {
            return Ok(None);
        };
        Ok(Some(ready))
    }
}

/// These imports are implemented by the interpreter, not a registry service.
/// This is the same provider selected by bytecode dispatch, never a script grant.
pub(super) fn owns_native(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "__mutate_integer"
            | "swap"
            | "swapvar"
            | "setbit"
            | "clearbit"
            | "invertbit"
            | "split"
            | "getnum"
            | "erdname"
            | "__indexbyname"
            | "setvar"
            | "getvar"
            | "getvars"
            | "__encodetouni_result"
            | "strjoin"
            | "arrayremove"
            | "arrayshift"
            | "arraysort"
            | "arraycopy"
            | "varset"
            | "cvarset"
            | "arraymsort"
            | "arraymsortex"
            | "findelement"
            | "findlastelement"
            | "regexpmatch"
            | "sumarray"
            | "sumcarray"
            | "maxarray"
            | "maxcarray"
            | "minarray"
            | "mincarray"
            | "match"
            | "cmatch"
            | "inrangearray"
            | "inrangecarray"
            | "groupmatch"
            | "nosames"
            | "allsames"
            | "charanum"
            | "getchara"
            | "getspchara"
            | "existcsv"
            | "csvname"
            | "csvcallname"
            | "csvnickname"
            | "csvmastername"
            | "csvcstr"
            | "csvbase"
            | "csvabl"
            | "csvmark"
            | "csvexp"
            | "csvrelation"
            | "csvtalent"
            | "csvcflag"
            | "csvequip"
            | "csvjuel"
            | "findchara"
            | "findlastchara"
            | "addchara"
            | "addspchara"
            | "adddefchara"
            | "addvoidchara"
            | "delchara"
            | "delallchara"
            | "swapchara"
            | "copychara"
            | "addcopychara"
            | "pickupchara"
            | "sortchara"
            | "reset_stain"
    )
}

fn is_array_query(native_name: &str) -> bool {
    matches!(
        native_name,
        "sumarray"
            | "sumcarray"
            | "maxarray"
            | "maxcarray"
            | "minarray"
            | "mincarray"
            | "match"
            | "cmatch"
            | "inrangearray"
            | "inrangecarray"
            | "groupmatch"
            | "nosames"
            | "allsames"
    )
}

fn is_character_query(native_name: &str) -> bool {
    matches!(
        native_name,
        "charanum"
            | "getchara"
            | "getspchara"
            | "existcsv"
            | "csvname"
            | "csvcallname"
            | "csvnickname"
            | "csvmastername"
            | "csvcstr"
            | "csvbase"
            | "csvabl"
            | "csvmark"
            | "csvexp"
            | "csvrelation"
            | "csvtalent"
            | "csvcflag"
            | "csvequip"
            | "csvjuel"
            | "findchara"
            | "findlastchara"
    )
}
