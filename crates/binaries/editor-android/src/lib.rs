#![cfg(any(target_os = "android", target_os = "linux"))]
#![recursion_limit = "256"]
#![cfg_attr(not(target_os = "android"), allow(dead_code))]

mod benchmark;
#[cfg(target_os = "android")]
mod bridge;
mod input;
#[cfg(target_os = "android")]
mod localization;
#[cfg(target_os = "android")]
mod preview;
mod project;
mod runtime;
