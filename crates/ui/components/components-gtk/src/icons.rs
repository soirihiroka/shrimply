use std::sync::Once;

use gtk::{gio, glib};

const RESOURCE_PREFIX: &str = "/dev/shrimply/icons";

pub fn register_bundled() {
    static REGISTERED: Once = Once::new();
    REGISTERED.call_once(|| {
        let bytes = build_gresource(shrimply_components_core::icons::ALL);
        let resource = gio::Resource::from_data(&glib::Bytes::from_owned(bytes))
            .expect("bundled icon GResource must be valid");
        gio::resources_register(&resource);
    });
    if let Some(display) = gtk::gdk::Display::default() {
        let theme = gtk::IconTheme::for_display(&display);
        if !theme.resource_path().iter().any(|p| p == RESOURCE_PREFIX) {
            theme.add_resource_path(RESOURCE_PREFIX);
        }
    }
}

/// Pack `(name, svg)` pairs into an uncompressed GLib GVDB `GResource` under `RESOURCE_PREFIX`.
fn build_gresource<'a>(icons: &[(&'a str, &'a str)]) -> Vec<u8> {
    #[derive(Clone, Copy)]
    struct Entry<'a> {
        key: &'a str,
        parent: Option<usize>,
        svg: Option<&'a str>,
        hash: u32,
    }

    let mut entries = Vec::with_capacity(icons.len() + 4);
    for (parts, key, parent) in [
        (&["/"][..], "/", None),
        (&["/dev/"], "dev/", Some(0)),
        (&["/dev/shrimply/"], "shrimply/", Some(1)),
        (&[RESOURCE_PREFIX, "/"], "icons/", Some(2)),
    ] {
        entries.push(Entry {
            key,
            parent,
            svg: None,
            hash: djb2(parts),
        });
    }
    for &(name, svg) in icons {
        entries.push(Entry {
            key: name,
            parent: Some(3),
            svg: Some(svg),
            hash: djb2(&[RESOURCE_PREFIX, "/", name, ".svg"]),
        });
    }

    let n = entries.len();
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by_key(|&raw| (entries[raw].hash as usize) % n);

    let mut raw_to_sorted = vec![0u32; n];
    let mut children = vec![Vec::<u32>::new(); n];
    for (sorted, &raw) in order.iter().enumerate() {
        raw_to_sorted[raw] = sorted as u32;
        if let Some(parent) = entries[raw].parent {
            children[parent].push(sorted as u32);
        }
    }

    let root_start: u32 = 24;
    let table_len = 8 + 4 * n + 24 * n;
    let root_end = root_start + table_len as u32;
    let mut buf = Vec::with_capacity(root_end as usize + icons.len() * 512);
    buf.extend_from_slice(b"GVariant");
    buf.extend_from_slice(&0u32.to_le_bytes());
    buf.extend_from_slice(&0u32.to_le_bytes());
    buf.extend_from_slice(&root_start.to_le_bytes());
    buf.extend_from_slice(&root_end.to_le_bytes());
    buf.resize(root_end as usize, 0);

    let mut items = Vec::with_capacity(n * 24);
    for &raw in &order {
        let entry = entries[raw];
        let key_start = buf.len() as u32;
        buf.extend_from_slice(entry.key.as_bytes());
        if entry.svg.is_some() {
            buf.extend_from_slice(b".svg");
        }
        let key_size = (buf.len() as u32 - key_start) as u16;
        let (kind, value_start, value_end) = match entry.svg {
            None => {
                align(&mut buf, 4);
                let start = buf.len() as u32;
                for &child in &children[raw] {
                    buf.extend_from_slice(&child.to_le_bytes());
                }
                (b'L', start, buf.len() as u32)
            }
            Some(svg) => {
                align(&mut buf, 8);
                let start = buf.len() as u32;
                buf.extend_from_slice(&(svg.len() as u32).to_le_bytes());
                buf.extend_from_slice(&0u32.to_le_bytes());
                buf.extend_from_slice(svg.as_bytes());
                buf.extend_from_slice(b"\0\0(uuay)");
                (b'v', start, buf.len() as u32)
            }
        };
        items.extend_from_slice(&entry.hash.to_le_bytes());
        items.extend_from_slice(
            &entry
                .parent
                .map_or(u32::MAX, |p| raw_to_sorted[p])
                .to_le_bytes(),
        );
        items.extend_from_slice(&key_start.to_le_bytes());
        items.extend_from_slice(&key_size.to_le_bytes());
        items.push(kind);
        items.push(0);
        items.extend_from_slice(&value_start.to_le_bytes());
        items.extend_from_slice(&value_end.to_le_bytes());
    }

    let mut table = Vec::with_capacity(table_len);
    table.extend_from_slice(&0u32.to_le_bytes()); // n_bloom_words = 0
    table.extend_from_slice(&(n as u32).to_le_bytes());
    let mut cursor = 0usize;
    for bucket in 0..n {
        table.extend_from_slice(&(cursor as u32).to_le_bytes());
        while cursor < n && (entries[order[cursor]].hash as usize) % n == bucket {
            cursor += 1;
        }
    }
    table.extend_from_slice(&items);
    buf[root_start as usize..root_end as usize].copy_from_slice(&table);
    buf
}

fn djb2(parts: &[&str]) -> u32 {
    parts
        .iter()
        .flat_map(|s| s.bytes())
        .fold(5381u32, |hash, byte| {
            hash.wrapping_mul(33).wrapping_add(byte as i8 as i32 as u32)
        })
}

fn align(buf: &mut Vec<u8>, alignment: usize) {
    let rem = buf.len() % alignment;
    if rem != 0 {
        buf.resize(buf.len() + (alignment - rem), 0);
    }
}
