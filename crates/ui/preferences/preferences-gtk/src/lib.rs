#[cfg(target_os = "linux")]
mod acceleration;
pub mod page;
mod server;
mod trust;
pub use shrimply_preferences_core as store;
