//! SDK error type and typed BSON argument parsing for extension stages.

use std::fmt;

use bson::Document;
use serde::de::DeserializeOwned;

use crate::status;
use crate::sys::{MongoExtensionStatus, MONGO_EXTENSION_STATUS_RUNTIME_ERROR};

/// Errors produced by stage logic and BSON parsing, convertible to host status objects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtensionError {
    /// Invalid user input or stage arguments (e.g. wrong shape or disallowed value).
    BadValue(String),
    /// `serde` / BSON deserialization failed.
    FailedToParse(String),
    /// Extension runtime failure (logic error, I/O, etc.).
    Runtime(String),
    /// Error reported by the host with an explicit code.
    HostError {
        /// Host-specific error code (non-zero for failures).
        code: i32,
        /// Human-readable reason from the host.
        reason: String,
    },
}

impl fmt::Display for ExtensionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExtensionError::BadValue(s) => write!(f, "{s}"),
            ExtensionError::FailedToParse(s) => write!(f, "{s}"),
            ExtensionError::Runtime(s) => write!(f, "{s}"),
            ExtensionError::HostError { reason, .. } => write!(f, "{reason}"),
        }
    }
}

/// SDK result type for typed stage APIs (`Err` is always [`ExtensionError`]).
pub type Result<T> = std::result::Result<T, ExtensionError>;

impl ExtensionError {
    /// Status code returned to the host via [`into_raw_status`](ExtensionError::into_raw_status).
    pub fn status_code(&self) -> i32 {
        match self {
            ExtensionError::HostError { code, .. } => *code,
            ExtensionError::BadValue(_)
            | ExtensionError::FailedToParse(_)
            | ExtensionError::Runtime(_) => MONGO_EXTENSION_STATUS_RUNTIME_ERROR,
        }
    }

    /// Message stored on the host [`MongoExtensionStatus`](crate::sys::MongoExtensionStatus).
    pub fn status_reason(&self) -> String {
        match self {
            ExtensionError::BadValue(s) => format!("bad value: {s}"),
            ExtensionError::FailedToParse(s) => format!("failed to parse: {s}"),
            ExtensionError::Runtime(s) => s.clone(),
            ExtensionError::HostError { reason, .. } => reason.clone(),
        }
    }

    /// Converts into a heap-allocated [`MongoExtensionStatus`] for returning across `extern "C"`.
    pub fn into_raw_status(self) -> *mut MongoExtensionStatus {
        status::new_error_status(self.status_code(), self.status_reason())
    }
}

/// Deserialize stage arguments from a BSON document (typically the inner object of `{ $stage: <doc> }`).
pub fn parse_args<T: DeserializeOwned>(doc: Document) -> Result<T> {
    bson::from_document(doc).map_err(|e| ExtensionError::FailedToParse(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bson::doc;

    #[derive(serde::Deserialize, PartialEq, Debug)]
    struct N {
        n: i32,
    }

    #[test]
    fn parse_args_roundtrip() {
        let v: N = parse_args(doc! { "n": -1 }).expect("ok");
        assert_eq!(v.n, -1);
    }

    #[test]
    fn parse_args_invalid_type() {
        let e = parse_args::<N>(doc! { "n": "x" }).unwrap_err();
        assert!(matches!(e, ExtensionError::FailedToParse(_)));
    }
}
