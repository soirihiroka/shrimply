use std::borrow::Cow;
use std::sync::OnceLock;

/// Pack Rust-bundled SVGs into an uncompressed Qt RCC v1 resource tree under `:/shrimply/icons/`.
pub(crate) fn register_bundled() {
    static RCC: OnceLock<Vec<u8>> = OnceLock::new();
    let _ = RCC.get_or_init(|| {
        let rcc = build_rcc(shrimply_components_core::icons::ALL);
        let registered = unsafe { crate::ffi::register_rcc_resource(rcc.as_ptr()) };
        assert!(registered, "bundled Qt icon RCC must register");
        rcc
    });
}

fn build_rcc<'a>(icons: &[(&'a str, &'a str)]) -> Vec<u8> {
    let mut files: Vec<(String, u32, Cow<'a, [u8]>)> = icons
        .iter()
        .map(|&(name, svg)| {
            let filename = format!("{name}.svg");
            let hash = qt_hash(&filename);
            // Strip GTK4 GPA url() fallback prefix that Qt's SVG renderer rejects.
            let bytes = if svg.contains("gpa:foreground") {
                Cow::Owned(
                    svg.replace("url(&quot;#gpa:foreground&quot;) ", "")
                        .replace("url(\"#gpa:foreground\") ", "")
                        .into_bytes(),
                )
            } else {
                Cow::Borrowed(svg.as_bytes())
            };
            (filename, hash, bytes)
        })
        .collect();
    // Qt binary-searches directory children by qt_hash(name).
    files.sort_by(|left, right| left.1.cmp(&right.1).then_with(|| left.0.cmp(&right.0)));

    let mut data = Vec::with_capacity(files.iter().map(|(_, _, b)| b.len() + 4).sum());
    let mut data_offsets = Vec::with_capacity(files.len());
    for (_, _, bytes) in &files {
        data_offsets.push(data.len() as u32);
        data.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
        data.extend_from_slice(bytes);
    }

    let mut names = Vec::new();
    let push_name = |buf: &mut Vec<u8>, text: &str, hash: u32| -> u32 {
        let offset = buf.len() as u32;
        let units: Vec<u16> = text.encode_utf16().collect();
        buf.extend_from_slice(&(units.len() as u16).to_be_bytes());
        buf.extend_from_slice(&hash.to_be_bytes());
        for unit in units {
            buf.extend_from_slice(&unit.to_be_bytes());
        }
        offset
    };
    let shrimply_name = push_name(&mut names, "shrimply", qt_hash("shrimply"));
    let icons_name = push_name(&mut names, "icons", qt_hash("icons"));
    let mut file_name_offsets = Vec::with_capacity(files.len());
    for (filename, hash, _) in &files {
        file_name_offsets.push(push_name(&mut names, filename, *hash));
    }

    // Tree nodes (14 bytes each, big-endian):
    // 0: root "" -> child 1 ("shrimply")
    // 1: "shrimply" -> child 2 ("icons")
    // 2: "icons" -> children 3..3+N
    let mut tree = Vec::with_capacity((3 + files.len()) * 14);
    let push_dir = |buf: &mut Vec<u8>, name_offset: u32, count: u32, first_child: u32| {
        buf.extend_from_slice(&name_offset.to_be_bytes());
        buf.extend_from_slice(&0x0002u16.to_be_bytes());
        buf.extend_from_slice(&count.to_be_bytes());
        buf.extend_from_slice(&first_child.to_be_bytes());
    };
    push_dir(&mut tree, 0, 1, 1);
    push_dir(&mut tree, shrimply_name, 1, 2);
    push_dir(&mut tree, icons_name, files.len() as u32, 3);
    for (name_offset, data_offset) in file_name_offsets.into_iter().zip(data_offsets) {
        tree.extend_from_slice(&name_offset.to_be_bytes());
        tree.extend_from_slice(&0x0000u16.to_be_bytes());
        tree.extend_from_slice(&0u16.to_be_bytes()); // AnyCountry
        tree.extend_from_slice(&1u16.to_be_bytes()); // C language
        tree.extend_from_slice(&data_offset.to_be_bytes());
    }

    let data_offset: u32 = 20;
    let names_offset = data_offset + data.len() as u32;
    let tree_offset = names_offset + names.len() as u32;
    let mut rcc = Vec::with_capacity((tree_offset as usize) + tree.len());
    rcc.extend_from_slice(b"qres");
    rcc.extend_from_slice(&1u32.to_be_bytes());
    rcc.extend_from_slice(&tree_offset.to_be_bytes());
    rcc.extend_from_slice(&data_offset.to_be_bytes());
    rcc.extend_from_slice(&names_offset.to_be_bytes());
    rcc.extend_from_slice(&data);
    rcc.extend_from_slice(&names);
    rcc.extend_from_slice(&tree);
    rcc
}

fn qt_hash(text: &str) -> u32 {
    let mut hash = 0u32;
    for unit in text.encode_utf16() {
        hash = (hash << 4).wrapping_add(u32::from(unit));
        let top = hash & 0xf000_0000;
        if top != 0 {
            hash ^= top >> 23;
        }
        hash &= !top;
    }
    hash
}
