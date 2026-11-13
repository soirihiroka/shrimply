use sanim_core::{Geometry, Property, Unit};
use sanim_lang::{Engine, MemorySources, Sources};
use std::sync::Arc;

#[test]
fn images_are_watched_typed_elements_with_layout_and_shared_encoded_assets() {
    let mut files = MemorySources::default();
    files.insert(
        "sample.png",
        include_bytes!("../../../tests/fixtures/media-sample.png").as_slice(),
    );
    files.insert("main.sanim", "let a=<Image src=\"sample.png\" width={120pt}/>\nlet b=<Image src=\"sample.png\" width={120pt}/>\nadd(a)\nadd(b)\nwait(1s)");
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
    files.insert("main.sanim",format!("let first=<Image src=\"{url}\"/>\nlet second=<Image src=\"{url}\"/>\nadd(first)\nadd(second)\nwait(1s)"));
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
        format!("let image=<Image src=\"{url}\"/>\nadd(image)\nwait(1s)"),
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
    files.insert("main.sanim","mut width=50%\nlet image=<Image src=\"image.svg\" width={width}/>\nadd(image)\nwait(1s)\nwidth=75%\nwait(1s)");
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
