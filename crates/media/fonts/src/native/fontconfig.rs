//! Native hosts without a Fontconfig installation supply their font resources.
use std::{
    ffi::{CString, c_char, c_void},
    path::{Path, PathBuf},
    sync::OnceLock,
};

#[link(name = "fontconfig")]
unsafe extern "C" {
    fn FcConfigCreate() -> *mut c_void;
    fn FcConfigDestroy(config: *mut c_void);
    fn FcConfigParseAndLoadFromMemory(
        config: *mut c_void,
        xml: *const c_char,
        complain: i32,
    ) -> i32;
    fn FcConfigBuildFonts(config: *mut c_void) -> i32;
    fn FcConfigSetCurrent(config: *mut c_void) -> i32;
}

/// Call before creating native font maps or document-rendering workers.
/// Fontconfig retains the completed immutable configuration for all consumers,
/// including Pango and PDF rendering; no process environment is modified.
pub fn initialize(directories: &[PathBuf], cache: &Path) -> Result<(), String> {
    static CONFIGURATION: OnceLock<Result<(Vec<PathBuf>, PathBuf), String>> = OnceLock::new();
    let configured = CONFIGURATION
        .get_or_init(|| {
            if directories.is_empty() {
                return Err("Native font directories are unavailable".into());
            }
            std::fs::create_dir_all(cache)
                .map_err(|error| format!("Create font cache: {error}"))?;
            let mut xml = String::from("<fontconfig>");
            for directory in directories {
                xml.push_str("<dir>");
                xml.push_str(&xml_path(directory)?);
                xml.push_str("</dir>");
            }
            xml.push_str("<cachedir>");
            xml.push_str(&xml_path(cache)?);
            xml.push_str("</cachedir></fontconfig>");
            let xml = CString::new(xml).map_err(|_| "Font resource path contains a null byte")?;
            let config = unsafe { FcConfigCreate() };
            if config.is_null() {
                return Err("Create native font configuration".into());
            }
            let configured = unsafe {
                FcConfigParseAndLoadFromMemory(config, xml.as_ptr(), 0) != 0
                    && FcConfigBuildFonts(config) != 0
                    && FcConfigSetCurrent(config) != 0
            };
            // FcConfigSetCurrent retains its own reference on success.
            unsafe {
                FcConfigDestroy(config);
            }
            if !configured {
                return Err("Initialize native font resources".into());
            }
            Ok((directories.to_vec(), cache.to_owned()))
        })
        .as_ref()
        .map_err(Clone::clone)?;
    if configured.0 != directories || configured.1 != cache {
        return Err("Native font resources cannot change after initialization".into());
    }
    Ok(())
}

fn xml_path(path: &Path) -> Result<String, String> {
    Ok(path
        .to_str()
        .ok_or("Font resource path is not UTF-8")?
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;"))
}
