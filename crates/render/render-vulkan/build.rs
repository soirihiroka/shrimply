#[cfg(target_os = "linux")]
fn main() {
    use std::{collections::HashSet, fmt::Write, path::PathBuf};
    println!("cargo:rustc-check-cfg=cfg(ffmpeg_vulkan_queue_flags)");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
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
    }
    if std::env::var_os("CARGO_FEATURE_RENDERER").is_none() {
        return;
    }
    println!("cargo:rerun-if-env-changed=SPIRV_OPT");
    println!("cargo:rerun-if-env-changed=SPIRV_VAL");
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../render-core/shaders");
    let output = PathBuf::from(std::env::var_os("OUT_DIR").expect("Vulkan shader output"));
    let compiler = shrimply_slang_build::Compiler::new(&directory, &output);
    let mut generated = String::new();
    let mut object_abi = None;
    for (portable, target, table, suffix) in [
        (false, shrimply_slang_build::Target::Vulkan, "MODULES", ""),
        (
            true,
            shrimply_slang_build::Target::VulkanPortable,
            "PORTABLE_MODULES",
            "_portable",
        ),
        (
            true,
            shrimply_slang_build::Target::VulkanWordStorage,
            "WORD_STORAGE_MODULES",
            "_word_storage",
        ),
    ] {
        writeln!(generated, "static {table}: &[Module] = &[\n").unwrap();
        let mut names = HashSet::new();
        for module in include_str!("../render-core/shaders/kernels.txt")
            .lines()
            .chain(["video", "denoise"])
            .chain(std::env::var_os("CARGO_FEATURE_ANDROID_MEDIA").map(|_| "sample_image"))
        {
            let artifacts = compiler.compile_as(
                &directory.join(format!("{module}.slang")),
                &format!("{module}{suffix}"),
                target,
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
                        (matches!(
                            entry["name"].as_str(),
                            Some("convolution" | "render_background")
                        ) || (portable && entry["name"] == "composite_layered_image_layer"))
                            == buffered
                    })
                    .collect::<Vec<_>>();
                if selected.is_empty() {
                    continue;
                }
                let wrapper;
                let code = if buffered {
                    wrapper = compiler.compile_as(
                        &directory.join(format!("{module}_vulkan.slang")),
                        &format!("{module}_vulkan{suffix}"),
                        target,
                        &[],
                    );
                    &wrapper.filename
                } else if portable && module == "preview" {
                    let entry_names = selected
                        .iter()
                        .map(|entry| entry["name"].as_str().unwrap())
                        .collect::<Vec<_>>();
                    wrapper = compiler.compile_as(
                        &directory.join(format!("{module}.slang")),
                        &format!("{module}{suffix}_entries"),
                        target,
                        &entry_names,
                    );
                    &wrapper.filename
                } else {
                    &artifacts.filename
                };
                writeln!(generated, "Module {{ code: include_bytes!(concat!(env!(\"OUT_DIR\"), \"/{code}\")), kernels: &[").unwrap();
                for entry in selected {
                    let name = entry["name"].as_str().expect("kernel name");
                    assert!(
                        names.insert(name.to_owned()),
                        "Duplicate Vulkan kernel {name}"
                    );
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
        let object_entries = [
            "obj_compute",
            "obj_denoise_composite_compute",
            "obj_outline_distance_compute",
            "obj_outline_compute",
            "reflect_software",
        ];
        let objects = object_compiler.compile_as(
            &object_shaders.join("obj_software.slang"),
            &format!("obj_software{suffix}"),
            target,
            if matches!(target, shrimply_slang_build::Target::VulkanWordStorage) {
                &[]
            } else if portable {
                &["reflect_software"]
            } else {
                &object_entries
            },
        );
        let reflection: serde_json::Value =
            serde_json::from_slice(&objects.reflection).expect("Software OBJ reflection");
        // Mobile drivers compile the entire module even when only one entry is
        // selected. Keep unrelated kernels and reflection-only buffers out of it.
        let word_storage = matches!(target, shrimply_slang_build::Target::VulkanWordStorage);
        let entries = if word_storage {
            reflection["entryPoints"]
                .as_array()
                .expect("wavefront entrypoints")
                .iter()
                .filter_map(|entry| entry["name"].as_str())
                .filter(|name| {
                    !name.starts_with("reflect_")
                        && !matches!(*name, "obj_compute" | "obj_composite_upload_compute")
                })
                .collect::<Vec<_>>()
        } else {
            object_entries[..object_entries.len() - 1].to_vec()
        };
        let entry_modules = if portable {
            entries
                .iter()
                .map(|entry| {
                    object_compiler.compile_as(
                        &object_shaders.join("obj_software.slang"),
                        &format!("{entry}{suffix}"),
                        target,
                        &[entry],
                    )
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        let features = enum_values(&objects.abi, "SoftwareObjectFeature");
        let transport = features["Transport"];
        let grounds = features["Grounds"];
        let reflections = features["GroundReflections"];
        let mut options = String::new();
        for (name, value) in [
            ("TRANSPORT", transport),
            ("GROUNDS", grounds),
            ("GROUND_REFLECTIONS", reflections),
        ] {
            writeln!(options, "const {name}: u32 = {value};").unwrap();
        }
        std::fs::write(output.join("software_options.rs"), options)
            .expect("write reflected software options");
        if word_storage {
            let scratch = reflection["entryPoints"]
                .as_array()
                .unwrap()
                .iter()
                .find(|entry| entry["name"] == "reflect_wavefront")
                .expect("wavefront scratch ABI");
            let mut sizes = String::new();
            for parameter in scratch["parameters"].as_array().unwrap() {
                let constant = match parameter["name"].as_str().unwrap() {
                    "work" => "TRANSPORT_BYTES",
                    "sample" => "SAMPLE_BYTES",
                    "accumulation" => "ACCUMULATION_BYTES",
                    _ => panic!("unknown wavefront scratch"),
                };
                let size = parameter["binding"]["size"].as_u64().expect("scratch size");
                assert!(size > 0 && size.is_multiple_of(4), "word scratch layout");
                writeln!(sizes, "const {constant}: usize = {size};").unwrap();
            }
            std::fs::write(output.join("software_scratch.rs"), sizes)
                .expect("write reflected scratch sizes");
        }
        for module in if portable {
            entry_modules.as_slice()
        } else {
            std::slice::from_ref(&objects)
        } {
            let module_reflection: serde_json::Value =
                serde_json::from_slice(&module.reflection).expect("Software OBJ entry reflection");
            let code = if word_storage {
                prepare_object_shader(&output, &module.filename)
            } else {
                module.filename.clone()
            };
            writeln!(generated, "Module {{ code: include_bytes!(concat!(env!(\"OUT_DIR\"), \"/{code}\")), kernels: &[").unwrap();
            for entry in module_reflection["entryPoints"]
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
                writeln!(generated, "KernelLayout {{ name: {name:?}, entry: {name:?}, buffered: false, group: [{}, {}, {}], fields: &[Field {{ name: \"data\", offset: 0, size: 8, boolean: false, unsigned: false }}] }},", group[0], group[1], group[2]).unwrap();
            }
            generated.push_str("] },\n");
        }
        let reflected = reflection["entryPoints"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["name"] == "reflect_software")
            .expect("Software ABI entry");
        let mut assertions = String::from("const _: () = {\n");
        let mut uniform_packing = String::new();
        for parameter in reflected["parameters"]
            .as_array()
            .expect("Software ABI parameters")
        {
            let name = parameter["type"]["name"]
                .as_str()
                .expect("Software ABI struct name");
            if name == "SoftwareObjectOptions" {
                continue;
            }
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
                uniform_packing = packing;
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
        if let Some(reference) = &object_abi {
            assert_eq!(
                reference,
                &(assertions, uniform_packing),
                "Portable Vulkan must preserve uploaded OBJ structure layouts"
            );
        } else {
            std::fs::write(output.join("software_abi.rs"), &assertions)
                .expect("write Software OBJ ABI checks");
            std::fs::write(output.join("software_uniforms.rs"), &uniform_packing)
                .expect("write scalar scene packing");
            object_abi = Some((assertions, uniform_packing));
        }
        generated.push_str("];\n");
    }
    let mut reference_abi = None;
    for (target, suffix) in [
        (shrimply_slang_build::Target::Vulkan, ""),
        (shrimply_slang_build::Target::VulkanPortable, "_portable"),
        (
            shrimply_slang_build::Target::VulkanWordStorage,
            "_word_storage",
        ),
    ] {
        let abi = compiler.compile_as(
            &directory.join("reflection.slang"),
            &format!("reflection{suffix}"),
            target,
            &["reflect_abi"],
        );
        let reflected: serde_json::Value =
            serde_json::from_slice(&abi.reflection).expect("Vulkan ABI reflection");
        let word_storage = matches!(target, shrimply_slang_build::Target::VulkanWordStorage);
        if let Some(reference) = &reference_abi {
            assert_eq!(
                reflected_abi(reference, word_storage),
                reflected_abi(&reflected, word_storage),
                "Portable Vulkan must preserve uploaded structure layouts"
            );
        } else {
            reference_abi = Some(reflected.clone());
        }
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
        let layer_size = layer["binding"]["size"]
            .as_u64()
            .expect("layer structure size");
        writeln!(generated, "assert!(std::mem::size_of::<shrimply_render_core::Nv12LayerParams>() == {layer_size}usize.next_multiple_of(std::mem::align_of::<shrimply_render_core::Nv12LayerParams>()), \"Vulkan layer ABI stride\");").unwrap();
        if word_storage {
            for (field, byte) in [("sample_method", 1), ("address_mode", 2), ("kind", 3)] {
                writeln!(generated, "assert!(std::mem::offset_of!(shrimply_render_core::Nv12LayerParams, {field}) == std::mem::offset_of!(shrimply_render_core::Nv12LayerParams, blend_mode) + {byte}, \"Packed Vulkan layer byte offset\");").unwrap();
            }
        }
        generated.push_str("};\n");
    }
    std::fs::write(output.join("kernels.rs"), generated).expect("write Vulkan kernel layouts");
}

#[cfg(target_os = "linux")]
fn reflected_abi(reflection: &serde_json::Value, word_storage: bool) -> serde_json::Value {
    let parameters = reflection["entryPoints"][0]["parameters"]
        .as_array()
        .expect("ABI parameters");
    serde_json::Value::Array(
        parameters
            .iter()
            .map(|parameter| {
                let ty = parameter["type"]["name"].as_str().unwrap_or_default();
                let mut binding = parameter["binding"].clone();
                if word_storage && ty == "LayerCompositeParams" {
                    // Reflection reports live bytes, while array/upload strides include
                    // the existing trailing pointer-alignment padding in both layouts.
                    binding["size"] = binding["size"]
                        .as_u64()
                        .expect("composite size")
                        .next_multiple_of(size_of::<u64>() as u64)
                        .into();
                }
                if word_storage && parameter["name"] == "layer_blend_mode" {
                    binding["size"] = 4.into();
                }
                let fields = parameter["type"]["fields"].as_array().map(|fields| {
                    fields
                        .iter()
                        .filter_map(|field| {
                            let name = field["name"].as_str().expect("ABI field name");
                            if word_storage
                                && ty == "Nv12LayerParams"
                                && matches!(name, "sample_method" | "address_mode" | "kind")
                            {
                                return None;
                            }
                            let mut binding = field["binding"].clone();
                            if word_storage
                                && ((ty == "Nv12LayerParams" && name == "blend_mode")
                                    || (ty == "LayerCompositeParams" && name == "mode"))
                            {
                                binding["size"] = 4.into();
                            }
                            Some(serde_json::json!([field["name"], binding]))
                        })
                        .collect::<Vec<_>>()
                });
                serde_json::json!([parameter["name"], binding, fields])
            })
            .collect(),
    )
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
    let unsigned = parameter["type"]["scalarType"] == "uint32";
    writeln!(
        output,
        "Field {{ name: {name:?}, offset: {offset}, size: {size}, boolean: {boolean}, unsigned: {unsigned} }},"
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

#[cfg(target_os = "linux")]
fn enum_values(abi: &[u8], name: &str) -> std::collections::BTreeMap<String, u32> {
    std::str::from_utf8(abi)
        .expect("shader ABI is UTF-8")
        .lines()
        .filter_map(|line| {
            let columns = line.split('\t').collect::<Vec<_>>();
            match columns.as_slice() {
                ["enum", ty, "uint32", variant, value] if *ty == name => Some((
                    (*variant).to_owned(),
                    value.parse().expect("unsigned shader option"),
                )),
                _ => None,
            }
        })
        .collect()
}

#[cfg(target_os = "linux")]
fn prepare_object_shader(output: &std::path::Path, source: &str) -> String {
    use std::process::Command;
    let filename = format!(
        "{}_prepared.spv",
        source.strip_suffix(".spv").expect("SPIR-V artifact")
    );
    let destination = output.join(&filename);
    let temporary = destination.with_extension("tmp.spv");
    let tool = std::env::var_os("SPIRV_OPT").unwrap_or_else(|| "spirv-opt".into());
    let status = Command::new(tool)
        .args([
            "--fold-spec-const-op-composite",
            "--eliminate-dead-branches",
            "--eliminate-dead-code-aggressive",
            "--eliminate-dead-functions",
            "--eliminate-local-single-block",
            "--eliminate-local-single-store",
            "--scalar-replacement=100",
            "--convert-local-access-chains",
            "--eliminate-local-single-block",
            "--eliminate-local-single-store",
            "--ssa-rewrite",
            "--ccp",
            "--simplify-instructions",
            "--eliminate-dead-branches",
            "--eliminate-dead-code-aggressive",
            "--eliminate-dead-functions",
            "--merge-return",
            "--merge-blocks",
            "--cfg-cleanup",
            "--simplify-instructions",
            "--eliminate-dead-code-aggressive",
        ])
        .arg(output.join(source))
        .arg("-o")
        .arg(&temporary)
        .status()
        .expect("Portable 3D shaders require SPIRV-Tools; install spirv-opt or set SPIRV_OPT");
    assert!(
        status.success(),
        "prepare software OBJ shader {source}: {status}"
    );
    let code = std::fs::read(&temporary).expect("read optimized object shader");
    std::fs::write(&temporary, shrimply_slang_build::order_declarations(&code))
        .expect("order object shader declarations");
    let validator = std::env::var_os("SPIRV_VAL").unwrap_or_else(|| "spirv-val".into());
    let status = Command::new(validator)
        .args(["--target-env", "vulkan1.1", "--scalar-block-layout"])
        .arg(&temporary)
        .status()
        .expect("Portable 3D shaders require SPIRV-Tools; install spirv-val or set SPIRV_VAL");
    assert!(
        status.success(),
        "validate software OBJ shader {source}: {status}"
    );
    std::fs::rename(&temporary, &destination).expect("install validated object shader");
    filename
}
