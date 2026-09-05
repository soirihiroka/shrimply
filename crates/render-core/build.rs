use std::{env, fs, path::PathBuf};

use serde_json::Value;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=shaders");
    println!("cargo:rerun-if-changed=../slang-build/compiler.cpp");
    println!("cargo:rerun-if-env-changed=SLANG_SOURCE_DIR");
    println!("cargo:rerun-if-env-changed=SLANG_BUILD_DIR");

    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let shader_directory = manifest.join("shaders");
    let source = shader_directory.join("reflection.slang");
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR"));
    let compiler = shrimply_slang_build::Compiler::new(&shader_directory, &output);
    let artifacts = compiler.compile(
        &source,
        shrimply_slang_build::Target::Host,
        &["reflect_abi"],
    );
    let reflection: Value = serde_json::from_slice(&artifacts.reflection)
        .unwrap_or_else(|error| panic!("parse compositor host reflection: {error}"));
    fs::write(
        output.join("abi.rs"),
        shrimply_slang_build::generate_abi(&reflection, &artifacts.abi),
    )
    .expect("write reflected compositor host ABI");

    let background = compiler.compile(
        &shader_directory.join("background_spirv.slang"),
        shrimply_slang_build::Target::Spirv,
        &["main"],
    );
    let reflection: Value =
        serde_json::from_slice(&background.reflection).expect("parse shared background reflection");
    let mut bindings = shrimply_slang_build::generate_module(
        "background_spirv",
        &background.filename,
        &reflection,
        &background.abi,
    );
    bindings.push_str("pub fn background_arguments<E>(uniforms: &background_spirv::BackgroundUniforms, mut field: impl FnMut(&str, &[u8]) -> Result<(), E>) -> Result<(), E> {\n");
    let uniforms = reflection["parameters"]
        .as_array()
        .expect("background parameters")
        .iter()
        .find(|parameter| parameter["name"] == "uniforms")
        .expect("background uniforms");
    let enum_fields = std::str::from_utf8(&background.abi)
        .expect("background ABI is UTF-8")
        .lines()
        .filter_map(|line| {
            let mut columns = line.split('\t');
            (columns.next()? == "enum-field").then(|| {
                (
                    columns.next().expect("enum struct"),
                    columns.next().expect("enum field"),
                )
            })
        })
        .collect::<std::collections::HashSet<_>>();
    background_fields(
        &mut bindings,
        &uniforms["type"]["elementType"],
        "uniforms",
        &enum_fields,
    );
    bindings.push_str("Ok(())\n}\n");
    fs::write(output.join("background.rs"), bindings).expect("write shared background ABI");
}

// Emit typed leaf values. Metal repacks these names using its own reflected
// offsets; Vulkan keeps the original reflected constant-buffer representation.
fn background_fields(
    output: &mut String,
    ty: &Value,
    prefix: &str,
    enum_fields: &std::collections::HashSet<(&str, &str)>,
) {
    use std::fmt::Write;
    for field in ty["fields"].as_array().expect("background struct fields") {
        let field_name = field["name"].as_str().expect("field name");
        let name = format!("{prefix}.{field_name}");
        let field_type = &field["type"];
        match field_type["kind"].as_str().expect("field kind") {
            "struct" => background_fields(output, field_type, &name, enum_fields),
            "scalar" => {
                let rust_type = match field_type["scalarType"].as_str().expect("scalar type") {
                    "float32" => "f32",
                    "uint32" => "u32",
                    kind => panic!("unsupported background scalar {kind}"),
                };
                if enum_fields.contains(&(ty["name"].as_str().expect("struct name"), field_name)) {
                    writeln!(
                        output,
                        "field({name:?}, &({name} as {rust_type}).to_ne_bytes())?;"
                    )
                    .unwrap();
                } else {
                    writeln!(output, "field({name:?}, &{name}.to_ne_bytes())?;").unwrap();
                }
            }
            "vector" => {
                assert_eq!(field_type["elementType"]["scalarType"], "float32");
                writeln!(
                    output,
                    "field({name:?}, {name}.map(f32::to_ne_bytes).as_flattened())?;"
                )
                .unwrap();
            }
            kind => panic!("unsupported background field {kind}"),
        }
    }
}
