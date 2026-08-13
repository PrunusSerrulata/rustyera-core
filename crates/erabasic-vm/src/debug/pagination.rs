//! Shared bounds validation for debugger pages.

use crate::VmError;

pub(super) fn page_bounds(cursor: Option<usize>, limit: usize) -> Result<(usize, usize), VmError> {
    if limit == 0 || limit > 1024 {
        return Err(VmError::InvalidArguments(
            "invalid debugger page size".into(),
        ));
    }
    Ok((cursor.unwrap_or(0), limit))
}
