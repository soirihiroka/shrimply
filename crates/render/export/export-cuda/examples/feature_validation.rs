//! Hardware validation of evaluated projects through preview and export.
//! Native project files are never written by this example.
#[cfg(target_os = "linux")]
#[path = "feature_validation/gpu.rs"]
mod gpu;
#[cfg(target_os = "linux")]
#[path = "feature_validation/run.rs"]
mod run;

#[cfg(target_os = "linux")]
fn main() -> Result<(), String> {
    run::run()
}

#[cfg(not(target_os = "linux"))]
fn main() {
    panic!("Intel feature validation requires Linux");
}
