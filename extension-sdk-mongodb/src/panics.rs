//! Catch unwinds at the FFI boundary so panics never cross into the MongoDB host.

use std::panic::{catch_unwind, AssertUnwindSafe};

/// Runs `f` and turns panics into `None` (caller should map to runtime error status).
pub fn ffi_boundary<T>(f: impl FnOnce() -> T) -> Option<T> {
    catch_unwind(AssertUnwindSafe(f)).ok()
}
