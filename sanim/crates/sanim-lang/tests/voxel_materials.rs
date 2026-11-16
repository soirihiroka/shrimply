use sanim_core::{Value, value_key::ValueKey};
use sanim_lang::{Engine, MemorySources};
fn prepare(source: &str) -> Result<sanim_lang::Prepared, sanim_core::Diagnostic> {
    let mut sources = MemorySources::default();
    sources.insert("main.sanim", format!("{source}\nAnimation()"));
    Engine::default().prepare("main.sanim", &sources, &Default::default())
}

#[test]
fn water_is_typed_pure_and_uses_linear_attenuation_with_coherent_wave_parameters() {
    prepare("let tint=VoxelMaterial(Color(\"blue3\"))").unwrap();
    let prepared=prepare(r##"let water=VoxelMaterial(transmission=1,ior=1.333,roughness=0.03,attenuation_color=Color("#808080"),attenuation_distance=8,wave_strength=0.08,wave_length=12,wave_speed=-1)"##).unwrap();
    let Value::VoxelMaterial(water) = &prepared.bindings["water"] else {
        panic!("water");
    };
    assert_eq!(water.optics().transmission, 1.);
    assert_eq!(water.optics().ior, 1.333);
    assert!((water.optics().attenuation_color[0] - 0.2158605).abs() < 1e-6);
    assert_eq!(water.optics().wave_speed, -1.);
    let mut engine = Engine::default();
    let mut sources = MemorySources::default();
    sources.insert(
        "main.sanim",
        r##"let _water=VoxelMaterial(transmission=1,ior=1.333,attenuation_color=Color("blue3"))
let _texture=read<VoxelTexture>("absent.png")
Animation()"##,
    );
    assert_eq!(
        engine
            .lint("main.sanim", &sources)
            .unwrap()
            .statistics
            .decoded_assets,
        0
    );
}

#[test]
fn optical_arguments_have_useful_type_and_range_errors() {
    for (source, message) in [
        (r##"let bad=VoxelMaterial(ior="water")"##, "Number"),
        ("let bad=VoxelMaterial(transmission=2)", "transmission"),
        ("let bad=VoxelMaterial(ior=0.8)", "ior"),
        (
            "let bad=VoxelMaterial(attenuation_distance=0)",
            "attenuation_distance",
        ),
        (
            r##"let bad=VoxelMaterial(attenuation_color=Color("#ffffff80"))"##,
            "opaque",
        ),
        (
            r##"let bad=VoxelMaterial(geometry="cross",transmission=1)"##,
            "closed cube",
        ),
        (r##"let bad=VoxelMaterial(geometry="triangle")"##, "cross"),
    ] {
        let error = prepare(source).unwrap_err();
        assert!(error.message.contains(message), "{error}");
        assert_eq!(error.span.line, 1);
    }
}

#[test]
fn pure_function_cache_keys_include_optics_and_geometry_without_changing_opaque_defaults() {
    let prepared=prepare(r##"
fn is_water(material:VoxelMaterial):Bool {repeat 100 {assert(true)};return material==VoxelMaterial(transmission=1,ior=1.333)}
let first=is_water(VoxelMaterial(transmission=1,ior=1.333))
let second=is_water(VoxelMaterial(transmission=1,ior=1.5))
let opaque=VoxelMaterial()
let card=VoxelMaterial(geometry="cross")
"##).unwrap();
    assert_eq!(prepared.bindings["first"], Value::Bool(true));
    assert_eq!(prepared.bindings["second"], Value::Bool(false));
    let opaque = serde_json::to_value(ValueKey(&prepared.bindings["opaque"])).unwrap();
    let card = serde_json::to_value(ValueKey(&prepared.bindings["card"])).unwrap();
    assert!(opaque["VoxelMaterial"].get("optics").is_none());
    assert!(opaque["VoxelMaterial"].get("geometry").is_none());
    assert_eq!(card["VoxelMaterial"]["geometry"], "cross");
}

#[test]
fn attenuation_preserves_weak_absorption_below_a_byte_of_color_precision() {
    let weak = sanim_core::Fraction::new(9999u64, 10000u64);
    let color = Value::Color(sanim_core::Color([
        weak,
        weak,
        weak,
        sanim_core::Fraction::from(1),
    ]));
    let linear = sanim_core::math::voxel::attenuation_color(&color).unwrap();
    assert!(linear[0] < 1.);
    let optics = sanim_voxel::OpticalProperties {
        attenuation_color: linear,
        ..Default::default()
    };
    assert!(optics.absorption()[0] > 0.);
}
