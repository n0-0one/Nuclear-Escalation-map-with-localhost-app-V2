//! Battle tracking engine for Nuclear Thunder Live.
//!
//! The engine has no user interface and performs no network access of its
//! own except for [`link`], which only ever talks to the game's read-only
//! web interface on this computer (127.0.0.1:8111).

pub mod convoy;
pub mod engine;
pub mod geo;
pub mod ident;
#[cfg(feature = "link")]
pub mod link;
pub mod pack;
pub mod roads;
pub mod sim;
pub mod telemetry;
pub mod world;

pub use engine::{Engine, Settings, Snapshot};
pub use pack::Pack;
