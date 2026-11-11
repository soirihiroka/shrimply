#[cfg(target_os = "linux")]
mod acceleration;
mod cli;
pub mod page;
mod server;
mod trust;
pub use shrimply_preferences_core as store;
