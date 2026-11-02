//! Shared Windows/Linux storage (M4): resolving Windows Known Folders, inspecting the Linux side,
//! and applying or undoing `~/Downloads`-style links *safely*.
//!
//! * [`wslconf`] reads the distribution's automount settings.
//! * [`distro`] is the small, allow-listed set of commands run inside the distribution.
//! * [`store`] records what was done so that it can be reversed.
//! * [`engine`] holds the safety logic.

pub mod distro;
pub mod engine;
pub mod store;
pub mod testing;
pub mod wslconf;

pub use distro::{DistroOps, OpsError, WslDistroOps};
pub use engine::{Ctx, MappingState, MappingStatus, Offer, Resolution};
pub use store::{MappingRecord, MappingStore, Replaced};
pub use wslconf::{parse_wsl_conf, WslConf};
