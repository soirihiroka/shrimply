#[cfg(target_os = "linux")]
fn main() {
    use std::{fmt::Write, path::PathBuf};
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../render-core/shaders");
    let output = PathBuf::from(std::env::var_os("OUT_DIR").expect("Vulkan shader output"));
    let compiler = shrimply_slang_build::Compiler::new(&directory, &output);
    let mut generated = String::from("static MODULES: &[Module] = &[\n");
    for module in include_str!("../render-core/shaders/kernels.txt")
        .lines()
        .chain(["video"])
    {
        let artifacts = compiler.compile(
            &directory.join(format!("{module}.slang")),
            shrimply_slang_build::Target::Vulkan,
            &[],
        );
        let reflection: serde_json::Value =
            serde_json::from_slice(&artifacts.reflection).expect("Slang Vulkan reflection");
        writeln!(
            generated,
            "Module {{ code: include_bytes!(concat!(env!(\"OUT_DIR\"), \"/{}\")), kernels: &[",
            artifacts.filename
        )
        .unwrap();
        for entry in reflection["entryPoints"]
            .as_array()
            .expect("compute entries")
        {
            let name = entry["name"].as_str().expect("kernel name");
            let group = entry["threadGroupSize"]
                .as_array()
                .expect("compute group size");
            writeln!(
                generated,
                "KernelLayout {{ name: {name:?}, group: [{}, {}, {}], fields: &[",
                group[0], group[1], group[2]
            )
            .unwrap();
            for parameter in entry["parameters"].as_array().expect("kernel parameters") {
                if parameter["binding"]["kind"] == "uniform" {
                    write_fields(&mut generated, parameter, "", 0);
                }
            }
            generated.push_str("] },\n");
        }
        generated.push_str("] },\n");
    }
    generated.push_str("];\n");
    let abi = compiler.compile(
        &directory.join("reflection.slang"),
        shrimply_slang_build::Target::Vulkan,
        &["reflect_abi"],
    );
    let reflected: serde_json::Value =
        serde_json::from_slice(&abi.reflection).expect("Vulkan ABI reflection");
    let layer = reflected["entryPoints"][0]["parameters"]
        .as_array()
        .expect("ABI parameters")
        .iter()
        .find(|parameter| parameter["type"]["name"] == "Nv12LayerParams")
        .expect("Vulkan layer ABI");
    generated.push_str("const _: () = {\n");
    for field in layer["type"]["fields"].as_array().expect("layer fields") {
        let name = field["name"].as_str().expect("layer field name");
        let offset = field["binding"]["offset"]
            .as_u64()
            .expect("layer field offset");
        writeln!(generated, "assert!(std::mem::offset_of!(shrimply_render_core::Nv12LayerParams, {name}) == {offset}, \"Vulkan layer ABI mismatch: {name}\");").unwrap();
    }
    generated.push_str("};\n");
    std::fs::write(output.join("kernels.rs"), generated).expect("write Vulkan kernel layouts");
}

#[cfg(target_os = "linux")]
fn write_fields(output: &mut String, parameter: &serde_json::Value, prefix: &str, base: u64) {
    use std::fmt::Write;
    let binding = &parameter["binding"];
    let name = format!(
        "{prefix}{}",
        parameter["name"].as_str().expect("parameter name")
    );
    let offset = base + binding["offset"].as_u64().expect("uniform offset");
    let size = binding["size"].as_u64().expect("uniform size");
    let boolean = parameter["type"]["scalarType"] == "bool";
    writeln!(
        output,
        "Field {{ name: {name:?}, offset: {offset}, size: {size}, boolean: {boolean} }},"
    )
    .unwrap();
    if let Some(fields) = parameter["type"]["fields"].as_array() {
        for field in fields {
            write_fields(output, field, &format!("{name}."), offset);
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn main() {}
