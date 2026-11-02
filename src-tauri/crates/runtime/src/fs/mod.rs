//! File manager backend (M4): safe, typed file operations on the distribution's filesystem and on
//! the shared Windows folders. Tauri-free, so everything here is tested with real temporary
//! directories standing in for the distribution root and the Windows folders.
//!
//! * [`path`]     — validation of Linux paths and names
//! * [`space`]    — the three storage kinds, location resolution, link handling, containment
//! * [`list`]     — paged directory enumeration
//! * [`ops`]      — create / rename / delete / copy / move with progress and cancellation
//! * [`filetype`] — classification and the "Open" policy

pub mod error;
pub mod filetype;
pub mod list;
pub mod ops;
pub mod path;
pub mod space;

#[cfg(test)]
pub(crate) mod testutil;

pub use error::{FsError, FsErrorKind, FsResult};
pub use list::{Entry, EntryKind, ListOptions, ListStore, ListingHead, Page, SortKey};
pub use ops::{Conflict, OpReport, Progress, Runner, TransferMode};
pub use space::{Location, Spaces, WindowsRoot};
