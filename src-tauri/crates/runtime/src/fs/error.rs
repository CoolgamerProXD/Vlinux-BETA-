//! Errors of the file layer. Each carries a machine-readable kind (the UI can react to it) and a
//! plain-language message (the UI can show it as is).

use std::fmt;
use std::io;

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FsErrorKind {
    NotFound,
    PermissionDenied,
    AlreadyExists,
    NotADirectory,
    IsADirectory,
    NotEmpty,
    /// Malformed path (relative, `..`, control characters, too long, ...).
    InvalidPath,
    /// Malformed file name for creating/renaming.
    InvalidName,
    /// The path would leave the places the file manager is allowed to reach.
    OutsideRoot,
    /// This place or operation is not available (for example no distribution is selected).
    Unavailable,
    /// The operation was refused on purpose (protected location, not confirmed, ...).
    Refused,
    Cancelled,
    Io,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FsError {
    pub kind: FsErrorKind,
    pub message: String,
}

impl FsError {
    pub fn new(kind: FsErrorKind, message: impl Into<String>) -> Self {
        Self { kind, message: message.into() }
    }

    /// Translate an I/O error into something a person can act on. `action` is a verb phrase
    /// ("open", "create", "delete"), `name` the item involved.
    pub fn from_io(e: &io::Error, action: &str, name: &str) -> Self {
        let (kind, message) = match e.kind() {
            io::ErrorKind::NotFound => (
                FsErrorKind::NotFound,
                format!("Can't {action} \"{name}\": it does not exist or can't be reached any more."),
            ),
            io::ErrorKind::PermissionDenied => (
                FsErrorKind::PermissionDenied,
                format!("Can't {action} \"{name}\": permission denied. You may not own it, or it is read-only."),
            ),
            io::ErrorKind::AlreadyExists => (
                FsErrorKind::AlreadyExists,
                format!("Can't {action} \"{name}\": something with that name already exists."),
            ),
            io::ErrorKind::DirectoryNotEmpty => {
                (FsErrorKind::NotEmpty, format!("Can't {action} \"{name}\": the folder is not empty."))
            }
            io::ErrorKind::NotADirectory => {
                (FsErrorKind::NotADirectory, format!("Can't {action} \"{name}\": it is not a folder."))
            }
            io::ErrorKind::IsADirectory => {
                (FsErrorKind::IsADirectory, format!("Can't {action} \"{name}\": it is a folder."))
            }
            _ => (FsErrorKind::Io, format!("Can't {action} \"{name}\": {e}")),
        };
        Self { kind, message }
    }
}

impl fmt::Display for FsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for FsError {}

pub type FsResult<T> = Result<T, FsError>;
