mod reflection;
mod slang;

pub use reflection::{generate_abi, generate_module};
pub use slang::{Artifacts, Compiler, Target, shader_sources};
