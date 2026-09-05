use std::{
    ffi::{CString, c_char, c_int},
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub enum Target {
    Spirv,
    Cuda,
    Metal,
    Host,
}

#[repr(C)]
struct CompileRequest {
    directory: *const c_char,
    module: *const c_char,
    target: Target,
    code_path: *const c_char,
    reflection_path: *const c_char,
    abi_path: *const c_char,
    entries: *const *const c_char,
    entry_count: usize,
}

unsafe extern "C" {
    fn shrimply_slang_compile(request: *const CompileRequest) -> c_int;
}

pub struct Artifacts {
    pub filename: String,
    pub reflection: Vec<u8>,
    pub abi: Vec<u8>,
}

pub struct Compiler {
    directory: PathBuf,
    output: PathBuf,
}

impl Compiler {
    pub fn new(directory: &Path, output: &Path) -> Self {
        println!("cargo:rerun-if-changed={}", directory.display());
        Self {
            directory: directory.to_owned(),
            output: output.to_owned(),
        }
    }

    pub fn compile(&self, source: &Path, target: Target, entries: &[&str]) -> Artifacts {
        let module = source
            .file_stem()
            .and_then(|name| name.to_str())
            .expect("Slang module filename must be UTF-8");
        let extension = match target {
            Target::Spirv => "spv",
            Target::Cuda => "cu",
            Target::Metal => "metal",
            Target::Host => "cpp",
        };
        let filename = format!("{module}.{extension}");
        let code = self.output.join(&filename);
        let reflection = self
            .output
            .join(format!("{module}.{extension}.reflection.json"));
        let abi = self.output.join(format!("{module}.{extension}.abi"));
        let strings = [
            self.directory.as_path(),
            Path::new(module),
            &code,
            &reflection,
            &abi,
        ]
        .map(|path| {
            CString::new(path.as_os_str().as_encoded_bytes()).expect("Slang path contains NUL")
        });
        let [directory, module_name, code_path, reflection_path, abi_path] = &strings;
        let entries: Vec<_> = entries
            .iter()
            .map(|entry| CString::new(*entry).expect("entry point contains NUL"))
            .collect();
        let entry_pointers: Vec<_> = entries.iter().map(|entry| entry.as_ptr()).collect();
        let request = CompileRequest {
            directory: directory.as_ptr(),
            module: module_name.as_ptr(),
            target,
            code_path: code_path.as_ptr(),
            reflection_path: reflection_path.as_ptr(),
            abi_path: abi_path.as_ptr(),
            entries: entry_pointers.as_ptr(),
            entry_count: entry_pointers.len(),
        };
        // The synchronous C++ API call borrows only the strings owned above.
        let result = unsafe { shrimply_slang_compile(&request) };
        assert_eq!(
            result, 0,
            "compile Slang module {module} for {target:?}; see compiler diagnostics"
        );
        Artifacts {
            filename,
            reflection: fs::read(&reflection)
                .unwrap_or_else(|error| panic!("read Slang reflection for {module}: {error}")),
            abi: fs::read(&abi)
                .unwrap_or_else(|error| panic!("read Slang ABI for {module}: {error}")),
        }
    }
}

pub fn shader_sources(directory: &Path) -> Vec<PathBuf> {
    let mut sources: Vec<_> = directory
        .read_dir()
        .unwrap_or_else(|error| panic!("read shader directory {}: {error}", directory.display()))
        .map(|entry| entry.expect("read shader directory entry").path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "slang")
        })
        .collect();
    sources.sort();
    assert!(
        !sources.is_empty(),
        "no .slang modules found in {}",
        directory.display()
    );
    sources
}
