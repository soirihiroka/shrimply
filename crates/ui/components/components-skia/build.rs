use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn collect_svgs(dir: &Path, out: &mut Vec<PathBuf>) {
    println!("cargo:rerun-if-changed={}", dir.display());
    let mut entries = fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("read icon directory {}: {error}", dir.display()))
        .map(|entry| entry.expect("read icon entry"))
        .collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.path());
    for entry in entries {
        let file_type = entry.file_type().expect("read icon file type");
        let path = entry.path();
        if file_type.is_dir() {
            collect_svgs(&path, out);
        } else if file_type.is_file()
            && path.extension().is_some_and(|extension| extension == "svg")
        {
            out.push(path);
        }
    }
}

fn main() {
    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let repo_root = manifest_dir.join("../../../..");
    let custom_dir = repo_root.join("assets/icons");
    let adwaita_dir = repo_root.join("external/adwaita-icon-theme/Adwaita/symbolic");
    assert!(
        adwaita_dir.is_dir(),
        "Missing Adwaita symbolic icons at {}. Run git submodule update --init external/adwaita-icon-theme.",
        adwaita_dir.display()
    );

    let mut custom_icons = Vec::new();
    collect_svgs(&custom_dir, &mut custom_icons);

    let mut adwaita_icons = Vec::new();
    collect_svgs(&adwaita_dir, &mut adwaita_icons);

    let mut names = HashSet::new();
    let mut generated =
        String::from("fn icon_svg(name: &str) -> &'static str {\n    match name {\n");
    for (strict, paths) in [(true, custom_icons), (false, adwaita_icons)] {
        for path in paths {
            let file_name = path
                .file_name()
                .and_then(|name| name.to_str())
                .expect("symbolic icon filename must be UTF-8");
            let name = file_name.strip_suffix(".svg").expect("SVG suffix");
            if !names.insert(name.to_owned()) {
                assert!(!strict, "duplicate custom symbolic icon {name}");
                continue;
            }
            let relative = path
                .strip_prefix(&repo_root)
                .expect("icon inside repo root")
                .to_str()
                .expect("UTF-8 icon path")
                .replace('\\', "/");
            generated.push_str(&format!(
                "        {name:?} => include_str!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/../../../../{relative}\")),\n"
            ));
        }
    }
    generated.push_str("        _ => panic!(\"unknown symbolic icon: {name}\"),\n    }\n}\n");

    let output = PathBuf::from(env::var_os("OUT_DIR").expect("build output directory"));
    fs::write(output.join("icons.rs"), generated).expect("write icon catalog");
}
