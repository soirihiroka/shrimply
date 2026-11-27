use sanim_core::{Geometry, Property, Unit};
use sanim_lang::{Engine, MemorySources, Sources};
use std::sync::Arc;

#[test]
fn generated_texture_pixels_are_shared_by_images_and_voxel_materials() {
    let mut files = MemorySources::default();
    files.insert(
        "main.sanim",
        r#"
let texture=Texture(size=[2,1],pixels=[Color("red"),Color("blue")])
let image=<Image src={texture} width={120pt}/>
let material=VoxelMaterial(texture=texture)
Animation(children=[image])
wait(1s)
"#,
    );
    let prepared = Engine::default()
        .prepare("main.sanim", &files, &Default::default())
        .unwrap();
    let sanim_core::Value::Texture(texture) = &prepared.bindings["texture"] else {
        panic!("texture")
    };
    let sanim_core::Value::VoxelMaterial(material) = &prepared.bindings["material"] else {
        panic!("material")
    };
    assert!(Arc::ptr_eq(texture, material.faces()[0].as_ref().unwrap()));
    let image = prepared.animation.find("image").unwrap();
    let Geometry::Media {
        asset,
        width,
        height,
    } = image.geometry.as_ref()
    else {
        panic!("image")
    };
    let sanim_core::media::Source::Texture(source) = asset.source() else {
        panic!("pixel source")
    };
    assert!(Arc::ptr_eq(source, texture));
    assert_eq!((*width, *height), (120.into(), 60.into()));
    assert_eq!(prepared.statistics.decoded_assets, 0);
}

#[test]
fn generated_texture_signals_update_image_pixels_and_preserve_fixed_display_size() {
    let mut files = MemorySources::default();
    files.insert(
        "main.sanim",
        r#"
mut hue=0deg
sig texture=Texture(size=[2,1],fill=Hsl(hue,1,0.5))
let image=<Image src={texture} width={120pt} height={80pt}/>
Animation(children=[image])
animate(hue,to=120deg,duration=1s,ease="linear")
wait(1s)
"#,
    );
    let prepared = Engine::default()
        .prepare("main.sanim", &files, &Default::default())
        .unwrap();
    for (time, wanted) in [(0, [255, 0, 0, 255]), (1, [0, 255, 0, 255])] {
        let frame = prepared.animation.sample(time.into()).unwrap();
        let Geometry::Media {
            asset,
            width,
            height,
        } = frame[0].geometry.as_ref()
        else {
            panic!("image")
        };
        let sanim_core::media::Source::Texture(texture) = asset.source() else {
            panic!("texture")
        };
        assert_eq!(&texture.pixels()[..4], wanted);
        assert_eq!((*width, *height), (120.into(), 80.into()));
    }
}

#[test]
fn texture_image_factories_are_pure_while_file_reads_still_require_ref() {
    let mut files = MemorySources::default();
    files.insert(
        "main.sanim",
        r#"
fn image(texture:Texture):Element { return <Image src={texture}/> }
let texture=Texture(size=[1,1],fill=Color("red"))
let output=image(texture)
Animation(children=[output])
wait(1s)
"#,
    );
    Engine::default()
        .prepare("main.sanim", &files, &Default::default())
        .unwrap();
    files.insert(
        "main.sanim",
        r#"fn image():Element { return <Image src="file.png"/> }"#,
    );
    let error = Engine::default()
        .prepare("main.sanim", &files, &Default::default())
        .unwrap_err();
    assert_eq!(error.code, "S133");
    assert!(error.message.contains("<ref>"), "{error}");
}

#[cfg(feature = "media")]
#[test]
fn drawing_native_shapes_and_text_reuses_the_existing_renderer_and_pixels() {
    let mut files = MemorySources::default();
    files.insert(
        "main.sanim",
        r#"
let base=Texture(size=[128,64],fill=Color("blue3"))
let shape=<Box width={48pt} height={24pt} fill="red" x_align={Align.Center} y_align={Align.Center}/>
let painted=base.draw(shape)
let repeated=base.draw(shape)
let lettering=Texture(size=[128,64]).draw(<Label font_size={12pt} fill="light1">Sanim</Label>)
let decal=base.composite(lettering)
let material=VoxelMaterial(texture=decal)
let image=<Image src={decal}/>
Animation(children=[image])
wait(1s)
"#,
    );
    let prepared = Engine::default()
        .prepare("main.sanim", &files, &Default::default())
        .unwrap();
    let get = |name: &str| match &prepared.bindings[name] {
        sanim_core::Value::Texture(texture) => texture,
        _ => panic!("texture"),
    };
    assert!(Arc::ptr_eq(get("painted"), get("repeated")));
    let painted = get("painted");
    let pixel = |x: usize, y: usize| &painted.pixels()[(y * 128 + x) * 4..(y * 128 + x + 1) * 4];
    assert_eq!(pixel(64, 32), [255, 0, 0, 255]);
    assert_ne!(pixel(2, 2), [255, 0, 0, 255]);
    assert!(
        get("lettering")
            .pixels()
            .as_chunks::<4>()
            .0
            .iter()
            .any(|pixel| pixel[3] > 0)
    );
    assert!(
        get("lettering")
            .pixels()
            .as_chunks::<4>()
            .0
            .iter()
            .any(|pixel| pixel[3] == 0)
    );
    assert!(
        get("decal")
            .pixels()
            .as_chunks::<4>()
            .0
            .iter()
            .all(|pixel| pixel[3] == 255)
    );
    assert_eq!(prepared.statistics.decoded_assets, 0);
}

#[cfg(feature = "media")]
#[test]
#[ignore = "manual native Skia texture drawing cold/cache benchmark"]
fn native_texture_drawing_performance_benchmark() {
    use sanim_core::{
        Value,
        math::texture::{MAX_BYTES, Op, cache_stats},
    };
    use std::time::Instant;
    const COLD_DRAWS: usize = 32;
    const WARM_DRAWS: usize = 1_000;
    let mut files = MemorySources::default();
    files.insert(
        "main.sanim",
        r#"
let base=Texture(size=[256,256],fill=Color("blue3"))
let shape=<Box width={128pt} height={72pt} radius={12pt} fill="orange3"/>
Animation()
"#,
    );
    let prepared = Engine::default()
        .prepare("main.sanim", &files, &Default::default())
        .unwrap();
    let Value::Texture(base) = &prepared.bindings["base"] else {
        panic!("Texture")
    };
    let shape = prepared.bindings["shape"].clone();
    sanim_render::install_texture_drawing();
    let mut args: imbl::OrdMap<String, Value> = [
        ("texture".to_string(), Value::Texture(base.clone())),
        ("content".to_string(), shape),
    ]
    .into_iter()
    .collect();
    let sources = (0..COLD_DRAWS)
        .map(|_| {
            Value::Texture(Arc::new(
                sanim_voxel::Texture::new(base.width(), base.height(), base.pixels().clone())
                    .unwrap(),
            ))
        })
        .collect::<Vec<_>>();
    let mut peak = 0;
    let started = Instant::now();
    let mut original = None;
    for source in sources {
        // New immutable source identity guarantees a real renderer miss without
        // copying the unchanged input pixels or clearing shared production caches.
        args.insert("texture".into(), source);
        let Value::Texture(output) = Op::Draw
            .evaluate(&args, |cost| {
                assert!(cost.work > 0);
                assert!(cost.bytes <= MAX_BYTES);
                peak = peak.max(cost.bytes);
                Ok(())
            })
            .unwrap()
        else {
            panic!("Texture")
        };
        assert_ne!(output.pixels(), base.pixels());
        original = Some(output);
    }
    let cold = started.elapsed();
    let original = original.unwrap();
    let before = cache_stats();
    let started = Instant::now();
    for _ in 0..WARM_DRAWS {
        let Value::Texture(output) = Op::Draw
            .evaluate(&args, |cost| {
                assert_eq!(
                    (cost.work, cost.bytes, cost.output_bytes),
                    (0, 0, 256 * 256 * 4)
                );
                Ok(())
            })
            .unwrap()
        else {
            panic!("Texture")
        };
        assert!(Arc::ptr_eq(&original, &output));
    }
    let warm = started.elapsed();
    let after = cache_stats();
    assert_eq!(after.rasterizations, before.rasterizations);
    assert!(after.bytes <= MAX_BYTES);
    assert_eq!(after.entries, before.entries);
    assert!((warm.as_nanos() / WARM_DRAWS as u128) < (cold.as_nanos() / COLD_DRAWS as u128));
    println!(
        "native_draw256x256 cold_count={COLD_DRAWS} cold_total_us={} cold_per_draw_us={} warm_count={WARM_DRAWS} warm_total_us={} warm_per_draw_ns={} warm_rasterizations={} cold_reserved_peak_bytes={peak} output_bytes={} cache_bytes={} cache_entries={}",
        cold.as_micros(),
        cold.as_micros() / COLD_DRAWS as u128,
        warm.as_micros(),
        warm.as_nanos() / WARM_DRAWS as u128,
        after.rasterizations - before.rasterizations,
        original.pixels().len(),
        after.bytes,
        after.entries
    );
}

#[cfg(feature = "media")]
#[test]
fn native_drawing_reserves_working_pixels_before_rendering_and_guards_cache_hits() {
    let mut files = MemorySources::default();
    files.insert(
        "main.sanim",
        r##"
let base=Texture(size=[64,64],fill=Color("#19374b"))
let painted=base.draw(<Box width={24pt} height={12pt} fill="orange3"/>)
Animation()
"##,
    );
    let mut constrained = Engine::default();
    constrained.limits.data.max_bytes = 40_000;
    let error = constrained
        .prepare("main.sanim", &files, &Default::default())
        .unwrap_err();
    assert!(
        error.message.contains("Texture.draw") && error.message.contains("byte budget"),
        "{error}"
    );
    Engine::default()
        .prepare("main.sanim", &files, &Default::default())
        .unwrap();
    constrained
        .prepare("main.sanim", &files, &Default::default())
        .unwrap();
    constrained.limits.data.max_bytes = 8_000;
    let error = constrained
        .prepare("main.sanim", &files, &Default::default())
        .unwrap_err();
    assert!(error.message.contains("byte budget"), "{error}");
}

#[cfg(feature = "media")]
#[test]
fn drawing_small_decals_accounts_for_full_bitmap_decoding_and_rejects_unbounded_svg() {
    for (bytes, reason) in [
        (
            include_bytes!("../../../tests/fixtures/media-sample.png").as_slice(),
            "host byte budget",
        ),
        (
            include_bytes!("../../../tests/fixtures/media-sample.svg").as_slice(),
            "cannot bound SVG decoding memory",
        ),
    ] {
        let mut files = MemorySources::default();
        files.insert("asset", bytes);
        files.insert(
            "main.sanim",
            r#"
let image=<Image src="asset" width={1pt} height={1pt}/>
let painted=Texture(size=[2,2]).draw(image)
Animation()
"#,
        );
        let mut engine = Engine::default();
        engine.limits.data.max_bytes = 100_000;
        let error = engine
            .prepare("main.sanim", &files, &Default::default())
            .unwrap_err();
        assert!(
            error.message.contains("Texture.draw") && error.message.contains(reason),
            "{error}"
        );
    }
}

#[test]
fn images_are_watched_typed_elements_with_layout_and_shared_encoded_assets() {
    let mut files = MemorySources::default();
    files.insert(
        "sample.png",
        include_bytes!("../../../tests/fixtures/media-sample.png").as_slice(),
    );
    files.insert("main.sanim", "let a=<Image src=\"sample.png\" width={120pt}/>\nlet b=<Image src=\"sample.png\" width={120pt}/>\nAnimation(children=[a])\nAnimation(children=[b])\nwait(1s)");
    let mut engine = Engine::default();
    let prepared = engine
        .prepare("main.sanim", &files, &Default::default())
        .unwrap();
    let a = prepared.animation.find("a").unwrap();
    let b = prepared.animation.find("b").unwrap();
    let Geometry::Media {
        asset: first,
        width,
        height,
    } = a.geometry.as_ref()
    else {
        panic!("media")
    };
    assert_eq!((*width, *height), (120.into(), 72.into()));
    let Geometry::Media { asset: second, .. } = b.geometry.as_ref() else {
        panic!("media")
    };
    assert!(Arc::ptr_eq(first, second));
    assert_eq!(
        a.property(Property::Width).unwrap().number().unwrap().unit,
        Unit::Length
    );
    assert_eq!(prepared.statistics.decoded_assets, 1);
    files.insert(
        "sample.png",
        include_bytes!("../../../tests/fixtures/media-sample.svg").as_slice(),
    );
    let changed = engine
        .prepare("main.sanim", &files, &Default::default())
        .unwrap();
    let Geometry::Media { asset, .. } = changed.animation.find("a").unwrap().geometry.as_ref()
    else {
        panic!("media")
    };
    assert_ne!(asset.fingerprint(), first.fingerprint());
    assert_eq!(asset.kind, sanim_core::media::Kind::Svg);
}

#[test]
fn remote_image_urls_share_one_download_and_remain_pinned_source_dependencies() {
    use std::{
        io::{BufRead, Write},
        net::TcpListener,
    };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/native.svg", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        {
            let mut request = std::io::BufReader::new(&mut stream);
            loop {
                let mut line = String::new();
                let count = request.read_line(&mut line).unwrap();
                if count == 0 || line == "\r\n" {
                    break;
                }
            }
        }
        let body = include_bytes!("../../../tests/fixtures/media-sample.svg");
        write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: image/svg+xml\r\nCache-Control: max-age=60\r\nETag: \"pinned\"\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",body.len()).unwrap();
        stream.write_all(body).unwrap();
    });
    let mut files = MemorySources::default();
    files.insert("main.sanim",format!("let first=<Image src=\"{url}\"/>\nlet second=<Image src=\"{url}\"/>\nAnimation(children=[first])\nAnimation(children=[second])\nwait(1s)"));
    struct RemoteSources(MemorySources);
    impl Sources for RemoteSources {
        fn read(&self, path: &str) -> Result<Vec<u8>, String> {
            self.read_snapshot(path)
                .map(|(bytes, _)| bytes.as_ref().clone())
        }
        fn read_snapshot(&self, path: &str) -> Result<(Arc<Vec<u8>>, String), String> {
            if shrimply_asset::remote::is_url(path) {
                let asset = shrimply_asset::remote::load(path, 4096)?;
                Ok((asset.bytes.clone(), asset.fingerprint.clone()))
            } else {
                self.0.read_snapshot(path)
            }
        }
    }
    let files = RemoteSources(files);
    let prepared = Engine::default()
        .prepare("main.sanim", &files, &Default::default())
        .unwrap();
    assert!(prepared.dependencies.contains_key(&url));
    assert_eq!(prepared.statistics.decoded_assets, 1);
    prepared.validate_dependencies(&files).unwrap();
    server.join().unwrap();
}

#[test]
fn custom_source_providers_keep_control_of_url_reads() {
    let url = "https://offline.invalid/mock.svg";
    let mut files = MemorySources::default();
    files.insert(
        url,
        include_bytes!("../../../tests/fixtures/media-sample.svg").as_slice(),
    );
    files.insert(
        "main.sanim",
        format!("let image=<Image src=\"{url}\"/>\nAnimation(children=[image])\nwait(1s)"),
    );
    let prepared = Engine::default()
        .prepare("main.sanim", &files, &Default::default())
        .unwrap();
    assert!(prepared.dependencies.contains_key(url));
    prepared.validate_dependencies(&files).unwrap();
}

#[test]
fn relative_image_width_and_signal_changes_preserve_intrinsic_aspect_ratio() {
    let mut files = MemorySources::default();
    files.insert(
        "image.svg",
        include_bytes!("../../../tests/fixtures/media-sample.svg").as_slice(),
    );
    files.insert("main.sanim","mut width=50%\nlet image=<Image src=\"image.svg\" width={width}/>\nAnimation(children=[image])\nwait(1s)\nanimate(width,to=75%,duration=0s)\nwait(1s)");
    let prepared = Engine::default()
        .prepare("main.sanim", &files, &Default::default())
        .unwrap();
    for (time, wanted) in [(0u64, 100f64), (1, 150f64)] {
        let frame = prepared
            .animation
            .layout(time.into(), 200.into(), 120.into())
            .unwrap();
        let Geometry::Media { width, height, .. } = frame[0].geometry.as_ref() else {
            panic!("image")
        };
        assert_eq!(sanim_core::math::to_f64(*width).unwrap(), wanted);
        assert_eq!(sanim_core::math::to_f64(*height).unwrap(), wanted * 0.6);
    }
}
