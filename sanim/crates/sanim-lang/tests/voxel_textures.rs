use sanim_core::Value;
use sanim_lang::{Engine, MemorySources};
use std::{sync::Arc, time::Instant};

fn prepare(source: &str) -> Result<sanim_lang::Prepared, sanim_core::Diagnostic> {
    let mut sources = MemorySources::default();
    sources.insert("main.sanim", format!("{source}\nAnimation()"));
    Engine::default().prepare("main.sanim", &sources, &Default::default())
}

#[test]
fn explicit_pixels_and_native_fields_use_the_same_row_major_rgba_and_cutout_rules() {
    let prepared = prepare(
        r#"
let pixels=Texture(size=[2,1],pixels=[Color("red"),Color("transparent")])
let field=Density(axis="x",offset=-0.5)
let mask=Density(axis="y",offset=-0.5)
let ramp=Texture(size=[2,2],field=field,colors=[Color("black"),Color("white")],mask=mask)
let middle=Color("black").mix(Color("white"),0.5,space="srgb")
"#,
    )
    .unwrap();
    let Value::Texture(pixels) = &prepared.bindings["pixels"] else {
        panic!("texture");
    };
    assert_eq!(pixels.pixels().as_ref(), [255, 0, 0, 255, 0, 0, 0, 0]);
    let Value::Texture(ramp) = &prepared.bindings["ramp"] else {
        panic!("texture");
    };
    assert_eq!(
        ramp.pixels().as_ref(),
        [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 255, 255, 255, 255, 255]
    );
    assert!(ramp.has_cutout());
    let Value::Color(middle) = &prepared.bindings["middle"] else {
        panic!("color");
    };
    assert_eq!(middle.rgba8().unwrap(), [128, 128, 128, 255]);
}

#[test]
fn texture_modes_sizes_alpha_and_sampling_fail_with_specific_errors() {
    for (source, reason) in [
        ("let t=Texture(size=[0,16],pixels=[])", "dimensions"),
        ("let t=Texture(size=[4097,1],pixels=[])", "dimensions"),
        ("let t=Texture(size=[2,2],pixels=[Color()])", "exactly 4"),
        (
            "let t=Texture(size=[1,1],pixels=[Color()],field=Density())",
            "cannot be combined",
        ),
        ("let t=Texture(size=[1,1],field=Density())", "colors"),
        (
            "let t=Texture(size=[1,1],field=Density(),colors=[])",
            "at least one",
        ),
        (
            r##"let t=Texture(size=[1,1],pixels=[Color("#ffffff80")]);let material=VoxelMaterial(texture=t)"##,
            "alpha",
        ),
        (
            r#"let t=Texture(size=[1,1],field=Density(),colors=[1])"#,
            "Color",
        ),
        (
            "let t=Texture(size=[1,1],field=Density(value=1).divide(0),colors=[Color()])",
            "non-finite",
        ),
        ("let c=Color().mix(Color(),2)", "0..=1"),
    ] {
        let error = prepare(source).unwrap_err();
        assert!(error.message.contains(reason), "{source}: {error}");
        assert_eq!(error.span.line, 1);
    }
}

#[test]
fn seeded_native_generation_is_pure_and_reuses_the_same_texture_storage() {
    let prepared = prepare(
        r#"
fn texture(seed:Number):Texture {
    return Texture(size=[16,16],field=Noise(seed=seed,axes="xy",scale=3).remap(-1,1,0,1),
        colors=[Color("black"),Color("white")])
}
let first=texture(71)
let again=texture(71)
let different=texture(72)
let material=VoxelMaterial(texture=first)
"#,
    )
    .unwrap();
    let (Value::Texture(first), Value::Texture(again), Value::Texture(different)) = (
        &prepared.bindings["first"],
        &prepared.bindings["again"],
        &prepared.bindings["different"],
    ) else {
        panic!("textures");
    };
    assert!(
        Arc::ptr_eq(first, again),
        "pure cache must retain the original immutable texture"
    );
    assert!(!Arc::ptr_eq(first, different));
    assert_ne!(first.pixels(), different.pixels());
    let Value::VoxelMaterial(material) = &prepared.bindings["material"] else {
        panic!("material");
    };
    assert!(
        material
            .faces()
            .iter()
            .all(|face| face.as_ref().is_some_and(|face| Arc::ptr_eq(first, face)))
    );
    assert_eq!(prepared.statistics.decoded_assets, 0);
}

#[test]
fn generation_reserves_native_work_and_output_bytes_before_allocation() {
    let mut sources = MemorySources::default();
    sources.insert("main.sanim", "let texture=Texture(size=[128,128],field=Noise(axes=\"xy\"),colors=[Color()])\nAnimation()");
    let mut engine = Engine::default();
    engine.limits.max_native_work = 100;
    let error = engine
        .prepare("main.sanim", &sources, &Default::default())
        .unwrap_err();
    assert!(error.message.contains("native work budget"), "{error}");
    engine.limits.max_native_work = 1_000_000;
    engine.limits.data.max_bytes = 1024;
    let error = engine
        .prepare("main.sanim", &sources, &Default::default())
        .unwrap_err();
    assert!(error.message.contains("byte budget"), "{error}");
}

#[test]
#[ignore = "manual native field texture generation benchmark"]
fn native_texture_generation_benchmark() {
    let source = r##"
let texture=Texture(size=[256,256],
    field=Noise(seed=71,axes="xy",scale=7,octaves=4).remap(-1,1,0,1),
    colors=[Color("#244E34"),Color("#7E9956")],
    mask=Noise(seed=19,axes="xy",scale=2).add(0.25))
Animation()
"##;
    let mut sources = MemorySources::default();
    sources.insert("main.sanim", source);
    let mut engine = Engine::default();
    let started = Instant::now();
    let first = engine
        .prepare("main.sanim", &sources, &Default::default())
        .unwrap();
    let cold = started.elapsed();
    sources.insert("main.sanim", format!("{source}\nlet caption=\"updated\""));
    let started = Instant::now();
    let second = engine
        .prepare("main.sanim", &sources, &Default::default())
        .unwrap();
    let warm = started.elapsed();
    let (Value::Texture(a), Value::Texture(b)) =
        (&first.bindings["texture"], &second.bindings["texture"])
    else {
        panic!("textures");
    };
    assert!(Arc::ptr_eq(a, b));
    assert_eq!(second.statistics.native_work, 0);
    assert_eq!(a.pixels().len(), 256 * 256 * 4);
    eprintln!(
        "native texture256x256 cold_us={} warm_suffix_us={} pixel_bytes={} cold_native_work={} suffix_native_work={}",
        cold.as_micros(),
        warm.as_micros(),
        a.pixels().len(),
        first.statistics.native_work,
        second.statistics.native_work
    );
}
