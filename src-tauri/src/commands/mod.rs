//! Tauri command surface. One sub-module per cluster of related commands.
//!
//! `lib.rs` imports these via `use commands::*;` so the command fns are
//! in scope for `tauri::generate_handler![]`. The re-exports look unused
//! inside this file but are load-bearing for the macro invocation.

#[allow(unused_imports)]
pub mod github;
#[allow(unused_imports)]
pub mod hermes;
#[allow(unused_imports)]
pub mod logs;
#[allow(unused_imports)]
pub mod plan;
#[allow(unused_imports)]
pub mod settings;
#[allow(unused_imports)]
pub mod updater;

#[allow(unused_imports)]
pub use github::*;
#[allow(unused_imports)]
pub use hermes::*;
#[allow(unused_imports)]
pub use logs::*;
#[allow(unused_imports)]
pub use plan::*;
#[allow(unused_imports)]
pub use settings::*;
#[allow(unused_imports)]
pub use updater::*;
