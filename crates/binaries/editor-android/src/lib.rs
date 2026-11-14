#![cfg(any(target_os = "android", target_os = "linux"))]
#![cfg_attr(not(target_os = "android"), allow(dead_code))]

mod benchmark;
#[cfg(target_os = "android")]
mod bridge;
mod input;
#[cfg(target_os = "android")]
mod localization;
mod math;
#[cfg(target_os = "android")]
mod preview;
mod project;
mod runtime;
