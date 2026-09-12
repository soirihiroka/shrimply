use std::{
    ffi::{CString, c_char, c_int},
    fs,
    path::{Path, PathBuf},
};
#[cfg(windows)]
use std::{os::windows::ffi::OsStrExt, sync::Once};
#[cfg(windows)]
use windows::{
    Win32::System::LibraryLoader::{
        AddDllDirectory, LOAD_LIBRARY_SEARCH_DEFAULT_DIRS, RemoveDllDirectory,
        SetDefaultDllDirectories,
    },
    core::PCWSTR,
};

#[cfg(windows)]
static DLL_SEARCH_INITIALIZED: Once = Once::new();

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

pub struct Artifacts {
    pub filename: String,
    pub reflection: Vec<u8>,
    pub abi: Vec<u8>,
}

pub struct Compiler {
    #[cfg(windows)]
    _slang: libloading::Library,
    #[cfg(windows)]
    dll_directory: usize,
    api: libloading::Library,
    directory: PathBuf,
    output: PathBuf,
}

impl Compiler {
    pub fn new(directory: &Path, output: &Path) -> Self {
        println!("cargo:rerun-if-changed={}", directory.display());
        println!("cargo:rerun-if-changed={}", crate::LIBRARY_DIR);
        // Load the bridge built with this crate; it links the pinned Slang C++ API.
        #[cfg(windows)]
        let (slang, dll_directory) = {
            DLL_SEARCH_INITIALIZED.call_once(|| unsafe {
                SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_DEFAULT_DIRS)
                    .expect("configure Windows DLL search directories");
            });
            let runtime = Path::new(crate::LIBRARY_DIR);
            let wide: Vec<_> = runtime.as_os_str().encode_wide().chain(Some(0)).collect();
            let dll_directory = unsafe { AddDllDirectory(PCWSTR::from_raw(wide.as_ptr())) };
            assert!(
                !dll_directory.is_null(),
                "register Slang DLL directory: {}",
                runtime.display()
            );
            let slang = unsafe { libloading::Library::new(runtime.join("slang.dll")) }
                .expect("load pinned Slang runtime");
            (slang, dll_directory as usize)
        };
        let api = unsafe { libloading::Library::new(env!("SHRIMPLY_SLANG_API")) }
            .expect("load Slang C++ API bridge");
        Self {
            #[cfg(windows)]
            _slang: slang,
            #[cfg(windows)]
            dll_directory,
            api,
            directory: directory.to_owned(),
            output: output.to_owned(),
        }
    }

    pub fn compile(&self, source: &Path, target: Target, entries: &[&str]) -> Artifacts {
        let module = source
            .file_stem()
            .and_then(|name| name.to_str())
            .expect("Slang module filename must be UTF-8");
        self.compile_as(source, module, target, entries)
    }

    pub fn compile_as(
        &self,
        source: &Path,
        artifact_name: &str,
        target: Target,
        entries: &[&str],
    ) -> Artifacts {
        let module = source
            .file_stem()
            .and_then(|name| name.to_str())
            .expect("Slang module filename must be UTF-8");
        assert!(
            !artifact_name.is_empty()
                && artifact_name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-'),
            "Slang artifact name must contain only ASCII letters, digits, '_' or '-'"
        );
        let extension = match target {
            Target::Spirv => "spv",
            Target::Cuda => "cu",
            Target::Metal => "metal",
            Target::Host => "cpp",
        };
        let filename = format!("{artifact_name}.{extension}");
        let code = self.output.join(&filename);
        let reflection = self
            .output
            .join(format!("{artifact_name}.{extension}.reflection.json"));
        let abi = self.output.join(format!("{artifact_name}.{extension}.abi"));
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
        let result = unsafe {
            let compile = self
                .api
                .get::<unsafe extern "C" fn(*const CompileRequest) -> c_int>(
                    b"shrimply_slang_compile\0",
                )
                .expect("load Slang compile function");
            compile(&request)
        };
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

#[cfg(windows)]
impl Drop for Compiler {
    fn drop(&mut self) {
        unsafe {
            RemoveDllDirectory(self.dll_directory as *const _).expect("remove Slang DLL directory");
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
