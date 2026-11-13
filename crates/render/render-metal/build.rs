#[cfg(target_os = "macos")]
fn main() {
    use objc2_foundation::NSString;
    use objc2_metal::{MTLCreateSystemDefaultDevice, MTLDevice, MTLLibrary};
    use std::{fmt::Write, path::PathBuf};

    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../render-core/shaders");
    let output = PathBuf::from(std::env::var_os("OUT_DIR").expect("Metal shader output"));
    let compiler = shrimply_slang_build::Compiler::new(&directory, &output);
    let device = MTLCreateSystemDefaultDevice().expect("Metal compiler device unavailable");
    let mut generated = String::from("static MODULES: &[Module] = &[\n");
    // Keep the same module set as render-cuda. Only target compilation differs.
    for module in include_str!("../render-core/shaders/kernels.txt")
        .lines()
        .chain(["video_metal"])
    {
        let source_path = directory.join(format!("{module}.slang"));
        let (artifacts, changed) = cached_artifacts(&directory, &output, module).map_or_else(
            || {
                (
                    compiler.compile(&source_path, shrimply_slang_build::Target::Metal, &[]),
                    true,
                )
            },
            |artifacts| (artifacts, false),
        );
        let source =
            std::fs::read_to_string(output.join(&artifacts.filename)).expect("read Metal source");
        let library = changed.then(|| {
            device
                .newLibraryWithSource_options_error(&NSString::from_str(&source), None)
                .unwrap_or_else(|error| panic!("compile Metal module {module}: {error}"))
        });
        let reflection: serde_json::Value =
            serde_json::from_slice(&artifacts.reflection).expect("Slang Metal reflection");
        writeln!(generated, "Module {{ name: {module:?}, source: include_str!(concat!(env!(\"OUT_DIR\"), \"/{}\")), kernels: &[", artifacts.filename).unwrap();
        for entry in reflection["entryPoints"]
            .as_array()
            .expect("compute entries")
        {
            assert_eq!(entry["stage"], "compute", "Metal entry must be compute");
            let name = entry["name"].as_str().expect("kernel name");
            if let Some(library) = &library {
                let metal_name = if name == "main" { "main_0" } else { name };
                library
                    .newFunctionWithName(&NSString::from_str(metal_name))
                    .unwrap_or_else(|| panic!("Metal module {module} omitted kernel {name}"));
            }
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
                let binding = &parameter["binding"];
                if binding["kind"] != "uniform" {
                    continue;
                }
                write_fields(&mut generated, parameter, "", 0);
            }
            generated.push_str("], textures: &[\n");
            for parameter in entry["parameters"].as_array().expect("kernel parameters") {
                if parameter["binding"]["kind"] == "shaderResource"
                    && parameter["type"]["baseShape"] == "texture2D"
                {
                    let name = parameter["name"].as_str().expect("texture name");
                    let index = parameter["binding"]["index"]
                        .as_u64()
                        .expect("texture binding");
                    writeln!(generated, "({name:?}, {index}),").unwrap();
                }
            }
            generated.push_str("] },\n");
        }
        generated.push_str("] },\n");
    }
    generated.push_str("];\n");
    std::fs::write(output.join("kernels.rs"), generated)
        .expect("write reflected Metal kernel layouts");
}

#[cfg(target_os = "macos")]
fn cached_artifacts(
    directory: &std::path::Path,
    output: &std::path::Path,
    module: &str,
) -> Option<shrimply_slang_build::Artifacts> {
    use std::time::SystemTime;

    let filename = format!("{module}.metal");
    let code = output.join(&filename);
    let reflection = output.join(format!("{module}.metal.reflection.json"));
    let abi = output.join(format!("{module}.metal.abi"));
    let generated = [&code, &reflection, &abi]
        .into_iter()
        .map(|path| path.metadata().ok()?.modified().ok())
        .collect::<Option<Vec<_>>>()?
        .into_iter()
        .min()?;
    let mut inputs = vec![directory.join(format!("{module}.slang"))];
    inputs.extend(
        directory
            .join("modules")
            .read_dir()
            .ok()?
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "slang")
            }),
    );
    let slang_build =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../gpu/slang/slang-build");
    inputs.push(slang_build.join("compiler.cpp"));
    inputs.push(slang_build.join("build.rs"));
    inputs.extend(
        std::path::Path::new(shrimply_slang_build::LIBRARY_DIR)
            .read_dir()
            .ok()?
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("libslang"))
            }),
    );
    let newest_input = inputs
        .into_iter()
        .map(|path| path.metadata().ok()?.modified().ok())
        .collect::<Option<Vec<SystemTime>>>()?
        .into_iter()
        .max()?;
    (generated >= newest_input).then(|| shrimply_slang_build::Artifacts {
        filename,
        reflection: std::fs::read(reflection).expect("read cached Metal reflection"),
        abi: std::fs::read(abi).expect("read cached Metal ABI"),
    })
}

#[cfg(target_os = "macos")]
fn write_fields(output: &mut String, parameter: &serde_json::Value, prefix: &str, base: u64) {
    use std::fmt::Write;
    let binding = &parameter["binding"];
    let name = format!(
        "{prefix}{}",
        parameter["name"].as_str().expect("parameter name")
    );
    let offset = base + binding["offset"].as_u64().expect("uniform offset");
    let size = binding["size"].as_u64().expect("uniform size");
    writeln!(
        output,
        "Field {{ name: {name:?}, offset: {offset}, size: {size} }},"
    )
    .unwrap();
    // Metal constant structs may pad vectors differently from CUDA's device ABI.
    // Named nested fields let all backends' callers supply the same values safely.
    if let Some(fields) = parameter["type"]["fields"].as_array() {
        for field in fields {
            write_fields(output, field, &format!("{name}."), offset);
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn main() {}
