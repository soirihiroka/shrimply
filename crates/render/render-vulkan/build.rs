#[cfg(target_os = "linux")]
fn main() {
    use std::{fmt::Write, path::PathBuf};
    println!("cargo:rustc-check-cfg=cfg(ffmpeg_vulkan_queue_flags)");
    let avutil = pkg_config::Config::new()
        .cargo_metadata(false)
        .probe("libavutil")
        .expect("FFmpeg libavutil development files");
    let major = avutil
        .version
        .split('.')
        .next()
        .expect("libavutil major version")
        .parse::<u32>()
        .expect("numeric libavutil major version");
    // FFmpeg 9 adds queue creation flags for vkGetDeviceQueue2.
    if major >= 61 {
        println!("cargo:rustc-cfg=ffmpeg_vulkan_queue_flags");
    }
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
        let entries = reflection["entryPoints"]
            .as_array()
            .expect("compute entries");
        for buffered in [false, true] {
            let selected = entries
                .iter()
                .filter(|entry| {
                    matches!(
                        entry["name"].as_str(),
                        Some("convolution" | "render_background")
                    ) == buffered
                })
                .collect::<Vec<_>>();
            if selected.is_empty() {
                continue;
            }
            let wrapper;
            let code = if buffered {
                wrapper = compiler.compile(
                    &directory.join(format!("{module}_vulkan.slang")),
                    shrimply_slang_build::Target::Vulkan,
                    &[],
                );
                &wrapper.filename
            } else {
                &artifacts.filename
            };
            writeln!(generated, "Module {{ code: include_bytes!(concat!(env!(\"OUT_DIR\"), \"/{code}\")), kernels: &[").unwrap();
            for entry in selected {
                let name = entry["name"].as_str().expect("kernel name");
                let entry_name = if buffered {
                    format!("{name}_vulkan")
                } else {
                    name.to_string()
                };
                let group = entry["threadGroupSize"]
                    .as_array()
                    .expect("compute group size");
                writeln!(generated, "KernelLayout {{ name: {name:?}, entry: {entry_name:?}, buffered: {buffered}, group: [{}, {}, {}], fields: &[", group[0], group[1], group[2]).unwrap();
                for parameter in entry["parameters"].as_array().expect("kernel parameters") {
                    if parameter["binding"]["kind"] == "uniform" {
                        write_fields(&mut generated, parameter, "", 0);
                    }
                }
                generated.push_str("] },\n");
            }
            generated.push_str("] },\n");
        }
    }
    let object_shaders = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../media/visual/3d/render-3d-core/shaders");
    let object_compiler = shrimply_slang_build::Compiler::new(&object_shaders, &output);
    let objects = object_compiler.compile(
        &object_shaders.join("obj_software.slang"),
        shrimply_slang_build::Target::Vulkan,
        &[
            "obj_compute",
            "obj_denoise_composite_compute",
            "obj_outline_distance_compute",
            "obj_outline_compute",
            "reflect_software",
        ],
    );
    let reflection: serde_json::Value =
        serde_json::from_slice(&objects.reflection).expect("Software OBJ reflection");
    writeln!(
        generated,
        "Module {{ code: include_bytes!(concat!(env!(\"OUT_DIR\"), \"/{}\")), kernels: &[",
        objects.filename
    )
    .unwrap();
    for entry in reflection["entryPoints"]
        .as_array()
        .expect("Software OBJ entries")
    {
        let name = entry["name"].as_str().expect("Software OBJ entry name");
        if name == "reflect_software" {
            continue;
        }
        let group = entry["threadGroupSize"]
            .as_array()
            .expect("Software OBJ group size");
        writeln!(generated, "KernelLayout {{ name: {name:?}, entry: {name:?}, buffered: false, group: [{}, {}, {}], fields: &[Field {{ name: \"data\", offset: 0, size: 8, boolean: false }}] }},", group[0], group[1], group[2]).unwrap();
    }
    generated.push_str("] },\n");
    let reflected = reflection["entryPoints"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["name"] == "reflect_software")
        .expect("Software ABI entry");
    let mut assertions = String::from("const _: () = {\n");
    for parameter in reflected["parameters"]
        .as_array()
        .expect("Software ABI parameters")
    {
        let name = parameter["type"]["name"]
            .as_str()
            .expect("Software ABI struct name");
        if name == "SceneUniforms" {
            let mut packing = String::from(
                "fn pack_scene_uniforms(value: &shrimply_render_3d_core::obj::SceneUniforms) -> Vec<u8> {\n",
            );
            let size = parameter["binding"]["size"]
                .as_u64()
                .expect("Scalar scene uniform size");
            writeln!(packing, "let mut output = vec![0; {size}];").unwrap();
            pack_fields(&mut packing, &parameter["type"], "value", "0", size);
            packing.push_str("output\n}\n");
            std::fs::write(output.join("software_uniforms.rs"), packing)
                .expect("write scalar scene packing");
            continue;
        }
        let rust_type = match name {
            "SceneUniforms" => "shrimply_render_3d_core::obj::SceneUniforms",
            "MeshInstance" => "shrimply_render_3d_core::obj::MeshInstance",
            _ => name,
        };
        for field in parameter["type"]["fields"]
            .as_array()
            .expect("Software ABI fields")
        {
            let field_name = field["name"].as_str().unwrap();
            let offset = field["binding"]["offset"].as_u64().unwrap();
            writeln!(assertions, "assert!(std::mem::offset_of!({rust_type}, {field_name}) == {offset}, \"Software OBJ ABI: {name}.{field_name}\");").unwrap();
        }
        let size = parameter["binding"]["size"].as_u64().unwrap();
        writeln!(assertions, "assert!(std::mem::size_of::<{rust_type}>() == {size}, \"Software OBJ ABI size: {name}\");").unwrap();
    }
    assertions.push_str("};\n");
    std::fs::write(output.join("software_abi.rs"), assertions)
        .expect("write Software OBJ ABI checks");
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

#[cfg(target_os = "linux")]
fn pack_fields(
    output: &mut String,
    ty: &serde_json::Value,
    expression: &str,
    offset: &str,
    size: u64,
) {
    use std::fmt::Write;
    match ty["kind"].as_str().expect("reflected uniform type") {
        "struct" => {
            for field in ty["fields"].as_array().expect("uniform struct fields") {
                let name = field["name"].as_str().unwrap();
                let field_offset = field["binding"]["offset"].as_u64().unwrap();
                let length = field["binding"]["size"].as_u64().unwrap();
                pack_fields(
                    output,
                    &field["type"],
                    &format!("{expression}.{name}"),
                    &offset.parse::<u64>().map_or_else(
                        |_| {
                            if field_offset == 0 {
                                offset.to_owned()
                            } else {
                                format!("({offset} + {field_offset})")
                            }
                        },
                        |base| (base + field_offset).to_string(),
                    ),
                    length,
                );
            }
        }
        "array" => {
            let count = ty["elementCount"].as_u64().expect("uniform array count");
            let variable = format!("index_{}", expression.matches('.').count());
            writeln!(output, "for {variable} in 0..{count} {{").unwrap();
            pack_fields(
                output,
                &ty["elementType"],
                &format!("{expression}[{variable}]"),
                &format!("({offset} + {variable} * {})", size / count),
                size / count,
            );
            output.push_str("}\n");
        }
        _ => {
            let end = offset.parse::<u64>().map_or_else(
                |_| format!("{offset} + {size}"),
                |base| (base + size).to_string(),
            );
            writeln!(output, "output[{offset}..{end}].copy_from_slice(bytes(std::slice::from_ref(&{expression}))); ").unwrap();
        }
    }
}
