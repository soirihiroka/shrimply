use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let icon_dir = manifest_dir.join("../../../assets/icons");
    println!("cargo:rerun-if-changed={}", icon_dir.display());

    let mut icons = fs::read_dir(&icon_dir)
        .expect("read symbolic icon directory")
        .map(|entry| entry.expect("read symbolic icon entry").path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "svg"))
        .collect::<Vec<_>>();
    icons.sort();

    let mut names = HashSet::new();
    let mut generated =
        String::from("fn icon_svg(name: &str) -> &'static str {\n    match name {\n");
    for path in icons {
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .expect("symbolic icon filename must be UTF-8");
        let name = file_name.strip_suffix(".svg").expect("SVG suffix");
        assert!(
            names.insert(name.to_owned()),
            "duplicate symbolic icon {name}"
        );
        generated.push_str(&format!(
            "        {name:?} => include_str!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/../../../assets/icons/{file_name}\")),\n"
        ));
    }
    generated.push_str("        _ => panic!(\"unknown symbolic icon: {name}\"),\n    }\n}\n");

    let output = PathBuf::from(env::var_os("OUT_DIR").expect("build output directory"));
    fs::write(output.join("icons.rs"), generated).expect("write icon catalog");
}
