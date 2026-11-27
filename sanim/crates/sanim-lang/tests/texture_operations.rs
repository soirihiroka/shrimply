use sanim_core::{Fraction, Geometry, Value};
use sanim_lang::{Engine, MemorySources};
use std::{sync::Arc, time::Instant};
fn prepare(source: &str) -> sanim_lang::Prepared {
    let mut sources = MemorySources::default();
    sources.insert("main.sanim", format!("{source}\nAnimation()"));
    Engine::default()
        .prepare("main.sanim", &sources, &Default::default())
        .unwrap()
}
fn texture(prepared: &sanim_lang::Prepared, name: &str) -> Arc<sanim_voxel::Texture> {
    if let Value::Texture(texture) = &prepared.bindings[name] {
        texture.clone()
    } else {
        panic!("Texture {name}")
    }
}
const PATTERN: &str = r##"let source=Texture(size=[2,2],pixels=[Color("#ff0000"),Color("#00ff00"),Color("#0000ff"),Color("#ffffff")])"##;

#[cfg(not(feature = "media"))]
#[test]
fn native_drawing_without_a_renderer_fails_with_an_actionable_feature_message() {
    let mut files = MemorySources::default();
    files.insert("main.sanim","let source=Texture(size=[4,4])\nlet painted=source.draw(<Box width={1pt} height={1pt}/>)\nAnimation()");
    let error = Engine::default()
        .prepare("main.sanim", &files, &Default::default())
        .unwrap_err();
    assert!(
        error.message.contains("native 2D rendering feature"),
        "{error}"
    );
    assert!(error.message.contains("CLI/editor"), "{error}");
}

#[test]
fn geometric_operations_preserve_pixels_and_the_original_immutable_value() {
    let prepared = prepare(&format!(
        r#"{PATTERN}
let crop=source.crop(min=[1,0],size=[1,2])
let scaled=source.resize(size=[4,4],filter="nearest")
let flip=source.flip(axes="x")
let turn=source.rotate(angle=90deg)
let padded=source.pad(size=[4,4])
let clipped=source.pad(size=[2,2],at=[-1,0])
let tiled=source.tile(size=[3,3])
let arbitrary=source.rotate(angle=30deg)
let dimensions=arbitrary.size()
let pixel=source.pixel(at=[0,0])
let identical=source.resize(size=[2,2]).crop(min=[0,0],size=[2,2]).brightness(factor=1)
"#
    ));
    let original = texture(&prepared, "source");
    assert_eq!(
        texture(&prepared, "crop").pixels().as_ref(),
        [0, 255, 0, 255, 255, 255, 255, 255]
    );
    assert_eq!(
        &texture(&prepared, "scaled").pixels()[..8],
        [255, 0, 0, 255, 255, 0, 0, 255]
    );
    assert_eq!(
        &texture(&prepared, "flip").pixels()[..8],
        [0, 255, 0, 255, 255, 0, 0, 255]
    );
    assert_eq!(
        &texture(&prepared, "turn").pixels()[..8],
        [0, 0, 255, 255, 255, 0, 0, 255]
    );
    assert_eq!(&texture(&prepared, "padded").pixels()[..4], [0; 4]);
    assert_eq!(
        &texture(&prepared, "clipped").pixels()[..8],
        [0, 255, 0, 255, 0, 0, 0, 0]
    );
    assert_eq!(
        &texture(&prepared, "tiled").pixels()[..12],
        [255, 0, 0, 255, 0, 255, 0, 255, 255, 0, 0, 255]
    );
    assert_eq!(texture(&prepared, "arbitrary").width(), 3);
    assert!(Arc::ptr_eq(&original, &texture(&prepared, "identical")));
    assert_eq!(&original.pixels()[..4], [255, 0, 0, 255]);
    let Value::Color(pixel) = &prepared.bindings["pixel"] else {
        panic!("Color")
    };
    assert_eq!(pixel.rgba8().unwrap(), [255, 0, 0, 255]);
}

#[test]
fn filtering_is_premultiplied_and_linear_without_transparent_color_fringes() {
    let prepared = prepare(
        r##"
let source=Texture(size=[2,1],pixels=[Color("red"),Color("#0000ff00")])
let blurred=source.blur(radius=1)
let scaled=source.resize(size=[9,3],filter="lanczos3")
let sharpened=source.sharpen(radius=1,amount=0.5,threshold=0)
let binary=blurred.cutout(threshold=0.5)
let material=VoxelMaterial(texture=binary)
"##,
    );
    for name in ["blurred", "scaled", "sharpened"] {
        let texture = texture(&prepared, name);
        for pixel in texture.pixels().as_chunks::<4>().0 {
            if pixel[3] > 0 {
                assert!(pixel[0] > 245, "{name}: {pixel:?}");
                assert_eq!(pixel[2], 0, "transparent blue must not contaminate {name}");
            }
        }
    }
    assert!(texture(&prepared, "blurred").has_partial_alpha());
    assert!(!texture(&prepared, "binary").has_partial_alpha());
}

#[test]
fn color_adjustments_compositing_masking_and_perceptual_blends_preserve_alpha() {
    let prepared = prepare(
        r##"
let red=Texture(size=[1,1],fill=Color("red"))
let blue=Texture(size=[1,1],fill=Color("blue"))
let translucent=Texture(size=[1,1],fill=Color("#ff000080"))
let canvas=Texture(size=[2,2])
let inverted=translucent.invert()
let gray=translucent.grayscale()
let dark=translucent.brightness(factor=0)
let contrast=red.contrast(factor=0)
let desaturated=red.saturation(factor=0)
let cyan=red.hue(angle=180deg)
let mixed=red.blend(other=blue,amount=0.5,space="srgb")
let over=blue.composite(over=translucent)
let multiplied=red.composite(over=blue,mode="multiply")
let masked=translucent.mask(mask=Texture(size=[1,1],fill=Color("#ffffff80")))
let replaced=translucent.with_alpha(alpha=1)
let hidden=Texture(size=[1,1],fill=Color("#ff000000")).composite(over=Texture(size=[1,1])).with_alpha(alpha=1)
let clipped=canvas.composite(over=Texture(size=[2,1],pixels=[Color("red"),Color("blue")]),at=[-1,0])
let smooth=Texture(size=[2,1],fill=Color("white"),alpha=Density(axis="x",offset=-0.5))
"##,
    );
    assert_eq!(texture(&prepared, "canvas").pixels().as_ref(), [0; 16]);
    assert_eq!(
        texture(&prepared, "inverted").pixels().as_ref(),
        [0, 255, 255, 128]
    );
    assert_eq!(texture(&prepared, "dark").pixels().as_ref(), [0, 0, 0, 128]);
    let gray = texture(&prepared, "gray");
    assert_eq!(gray.pixels()[0], gray.pixels()[1]);
    assert_eq!(gray.pixels()[1], gray.pixels()[2]);
    assert_eq!(gray.pixels()[3], 128);
    let gray = texture(&prepared, "desaturated");
    assert_eq!(gray.pixels()[0], gray.pixels()[1]);
    assert_eq!(
        texture(&prepared, "cyan").pixels().as_ref(),
        [0, 255, 255, 255]
    );
    assert_eq!(
        texture(&prepared, "mixed").pixels().as_ref(),
        [128, 0, 128, 255]
    );
    let over = texture(&prepared, "over");
    assert!((i32::from(over.pixels()[0]) - 188).abs() <= 1);
    assert!((i32::from(over.pixels()[2]) - 187).abs() <= 1);
    assert_eq!(over.pixels()[3], 255);
    assert_eq!(
        texture(&prepared, "multiplied").pixels().as_ref(),
        [0, 0, 0, 255]
    );
    assert_eq!(texture(&prepared, "masked").pixels()[3], 64);
    assert_eq!(texture(&prepared, "replaced").pixels()[3], 255);
    assert_eq!(
        texture(&prepared, "hidden").pixels().as_ref(),
        [255, 0, 0, 255]
    );
    assert_eq!(
        &texture(&prepared, "clipped").pixels()[..8],
        [0, 0, 255, 255, 0, 0, 0, 0]
    );
    assert_eq!(
        texture(&prepared, "smooth").pixels().as_ref(),
        [255, 255, 255, 0, 255, 255, 255, 255]
    );
}

#[test]
fn invalid_edits_and_smaller_host_budgets_are_not_bypassed_by_the_cache() {
    for (call, reason) in [
        ("source.crop(min=[1,1],size=[2,2])", "exceed"),
        ("source.resize(size=[0,2])", "dimensions"),
        ("source.resize(size=[2,2],filter=\"made-up\")", "nearest"),
        ("source.blur(radius=-1)", "nonnegative"),
        ("source.sharpen(threshold=256)", "255"),
        ("source.pixel(at=[2,0])", "outside"),
        ("source.blend(other=Texture(size=[1,1]))", "equal sizes"),
        ("source.mask(mask=Texture(size=[1,1]))", "equal sizes"),
        ("source.cutout(threshold=2)", "0..=1"),
        ("source.save(\"forbidden.png\")", "save"),
    ] {
        let mut sources = MemorySources::default();
        sources.insert(
            "main.sanim",
            format!("{PATTERN}\nlet invalid={call}\nAnimation()"),
        );
        let error = Engine::default()
            .prepare("main.sanim", &sources, &Default::default())
            .unwrap_err();
        assert!(error.message.contains(reason), "{call}: {error}");
    }
    let mut sources = MemorySources::default();
    sources.insert(
        "main.sanim",
        "let canvas=Texture(size=[127,129],fill=Color(\"#a17329\"))\nAnimation()",
    );
    let mut engine = Engine::default();
    engine.limits.data.max_bytes = 100_000;
    assert!(
        engine
            .prepare("main.sanim", &sources, &Default::default())
            .unwrap_err()
            .message
            .contains("byte budget")
    );
    let mut engine = Engine::default();
    engine
        .prepare("main.sanim", &sources, &Default::default())
        .unwrap();
    let mut constrained = Engine::default();
    constrained.limits.data.max_bytes = 32_000;
    assert!(
        constrained
            .prepare("main.sanim", &sources, &Default::default())
            .unwrap_err()
            .message
            .contains("byte budget")
    );
}

#[test]
fn unchanged_retained_inputs_reuse_generated_and_filtered_pixels_across_frame_times() {
    let prepared = prepare(
        r##"
mut tint=Color("#945B31")
sig texture=Texture(size=[64,64],fill=tint).blur(radius=2)
let image=Image(src=texture)
image.fade_in(duration=1ms)
wait(10s)
"##,
    );
    let mut previous = None;
    for time in [1, 2, 3, 6, 9] {
        let elements = prepared.animation.sample(Fraction::from(time)).unwrap();
        let Geometry::Media { asset, .. } = elements[0].geometry.as_ref() else {
            panic!("media")
        };
        let sanim_core::media::Source::Texture(texture) = asset.source() else {
            panic!("Texture")
        };
        if let Some(previous) = &previous {
            assert!(
                Arc::ptr_eq(previous, texture),
                "unchanged signals must not regenerate images per frame"
            );
        }
        previous = Some(texture.clone());
    }
}

#[test]
fn retained_texture_folding_and_animated_growth_respect_the_host_byte_budget() {
    let mut sources = MemorySources::default();
    sources.insert(
        "main.sanim",
        "sig canvas=Texture(size=[127,129])\nAnimation()",
    );
    // The same request must fail both before and after another host warms the cache.
    for warm in [false, true] {
        if warm {
            Engine::default()
                .prepare("main.sanim", &sources, &Default::default())
                .unwrap();
        }
        let mut constrained = Engine::default();
        constrained.limits.data.max_bytes = 32_000;
        let error = constrained
            .prepare("main.sanim", &sources, &Default::default())
            .unwrap_err();
        assert!(error.message.contains("byte budget"), "{error}");
    }
    let mut restored = Engine::default();
    restored
        .prepare("main.sanim", &sources, &Default::default())
        .unwrap();
    restored.limits.data.max_bytes = 32_000;
    assert!(
        restored
            .prepare("main.sanim", &sources, &Default::default())
            .unwrap_err()
            .message
            .contains("byte budget")
    );
    sources.insert(
        "main.sanim",
        r#"
mut side=8
sig canvas=Texture(size=[side,side])
let image=<Image src={canvas} width={64pt} height={64pt}/>
image.fade_in(duration=1ms)
animate(side,to=128,duration=1s,ease="linear")
wait(1s)
"#,
    );
    let mut constrained = Engine::default();
    constrained.limits.data.max_bytes = 32_000;
    let prepared = constrained
        .prepare("main.sanim", &sources, &Default::default())
        .unwrap();
    prepared
        .animation
        .sample(Fraction::new(1u64, 1000u64))
        .unwrap();
    let error = prepared.animation.sample(Fraction::from(2)).unwrap_err();
    assert!(error.contains("byte budget"), "{error}");
    let encoded = serde_json::to_vec(&prepared.animation).unwrap();
    let decoded: sanim_core::Animation = serde_json::from_slice(&encoded).unwrap();
    assert!(
        decoded
            .sample(Fraction::from(2))
            .unwrap_err()
            .contains("byte budget")
    );
}

#[test]
fn retained_image_chain_reserves_only_live_results_plus_each_temporary_peak() {
    let prepared = prepare(
        r##"
let base=Texture(size=[640,640],fill=Color("#C08B5F"))
mut side=704
sig transformed=base.resize(size=[side,side]).blur(radius=2).rotate(angle=30deg)
sig matte=Texture(size=transformed.size(),fill=Color("white"))
sig final=transformed.mask(mask=matte)
let image=<Image src={final} width={240pt} height={240pt}/>
image.fade_in(duration=1ms)
animate(side,to=832,duration=1s,ease="linear")
wait(1s)
"##,
    );
    let elements = prepared
        .animation
        .sample(Fraction::new(501u64, 1000u64))
        .unwrap();
    let Geometry::Media { asset, .. } = elements[0].geometry.as_ref() else {
        panic!("image")
    };
    let sanim_core::media::Source::Texture(texture) = asset.source() else {
        panic!("Texture")
    };
    assert_eq!([texture.width(), texture.height()], [1050, 1050]);
}

#[test]
#[ignore = "manual retained texture/frame, palette edit and bounded memory benchmark"]
fn retained_texture_performance_benchmark() {
    let prepared = prepare(
        r##"
mut tint=Color("#8C6F3A")
sig texture=Texture(size=[256,256],fill=tint).blur(radius=3)
let image=Image(src=texture)
image.fade_in(duration=1ms)
wait(10s)
"##,
    );
    let before = sanim_core::math::texture::cache_stats();
    let started = Instant::now();
    let mut original = None;
    for frame in 1..=300 {
        let elements = prepared
            .animation
            .sample(Fraction::new(frame as u64, 30u64))
            .unwrap();
        let Geometry::Media { asset, .. } = elements[0].geometry.as_ref() else {
            panic!("media")
        };
        let sanim_core::media::Source::Texture(texture) = asset.source() else {
            panic!("texture")
        };
        if let Some(original) = &original {
            assert!(Arc::ptr_eq(original, texture));
        } else {
            original = Some(texture.clone());
        }
    }
    let elapsed = started.elapsed();
    let after = sanim_core::math::texture::cache_stats();
    assert_eq!(before.rasterizations, after.rasterizations);
    eprintln!(
        "retained_texture256x256 frames=300 total_us={} per_frame_us={} rerasterizations={} cache_bytes={} cache_entries={}",
        elapsed.as_micros(),
        elapsed.as_micros() / 300,
        after.rasterizations - before.rasterizations,
        after.bytes,
        after.entries
    );
    let mut sources = MemorySources::default();
    let mut engine = Engine::default();
    let started = Instant::now();
    for index in 0..32 {
        sources.insert("main.sanim",format!("let texture=Texture(size=[512,512],fill=Rgb({},0.35,0.2)).blur(radius=2)\nAnimation()",index as f64/32.0));
        let prepared = engine
            .prepare("main.sanim", &sources, &Default::default())
            .unwrap();
        drop(prepared);
        let stats = sanim_core::math::texture::cache_stats();
        assert!(stats.bytes <= sanim_core::math::texture::MAX_BYTES);
        assert!(stats.entries <= 1024);
    }
    let edits = started.elapsed();
    let final_stats = sanim_core::math::texture::cache_stats();
    eprintln!(
        "texture512x512 edits=32 total_us={} cache_bytes={} cache_entries={}",
        edits.as_micros(),
        final_stats.bytes,
        final_stats.entries
    );
    let mut engine = Engine::default();
    let mut sources = MemorySources::default();
    let mut times = Vec::new();
    for index in 0..3 {
        sources.insert("main.sanim",format!("let texture=Texture(size=[256,256],field=Noise(seed=332,axes=\"xy\",scale=7).remap(-1,1,0,1),colors=[Rgb({},0.4,0.2),Color(\"white\")])\nAnimation()",index as f64/10.0));
        let started = Instant::now();
        engine
            .prepare("main.sanim", &sources, &Default::default())
            .unwrap();
        times.push(started.elapsed().as_micros());
    }
    eprintln!("texture256x256 same_noise_palette_edit_us={times:?}");
}
