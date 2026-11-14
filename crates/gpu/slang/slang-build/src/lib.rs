mod reflection;
mod slang;
mod spirv;

pub const LIBRARY_DIR: &str = env!("SHRIMPLY_SLANG_LIBRARY_DIR");

pub use reflection::{generate_abi, generate_metal_module, generate_module};
pub use slang::{Artifacts, Compiler, Target, shader_sources};
