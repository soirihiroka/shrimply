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
    struct Entry<'a> {
        full_path: String,
        key: String,
        parent_raw: Option<usize>,
        is_dir: bool,
        svg: &'a str,
        hash: u32,
        bucket: usize,
    }

    let mut entries = vec![
        Entry {
            full_path: "/".into(),
            key: "/".into(),
            parent_raw: None,
            is_dir: true,
            svg: "",
            hash: djb2("/"),
            bucket: 0,
        },
        Entry {
            full_path: "/dev/".into(),
            key: "dev/".into(),
            parent_raw: Some(0),
            is_dir: true,
            svg: "",
            hash: djb2("/dev/"),
            bucket: 0,
        },
        Entry {
            full_path: "/dev/shrimply/".into(),
            key: "shrimply/".into(),
            parent_raw: Some(1),
            is_dir: true,
            svg: "",
            hash: djb2("/dev/shrimply/"),
            bucket: 0,
        },
        Entry {
            full_path: format!("{RESOURCE_PREFIX}/"),
            key: "icons/".into(),
            parent_raw: Some(2),
            is_dir: true,
            svg: "",
            hash: djb2(&format!("{RESOURCE_PREFIX}/")),
            bucket: 0,
        },
    ];
    for &(name, svg) in icons {
        let key = format!("{name}.svg");
        let full_path = format!("{RESOURCE_PREFIX}/{key}");
        let hash = djb2(&full_path);
        entries.push(Entry {
            full_path,
            key,
            parent_raw: Some(3),
            is_dir: false,
            svg,
            hash,
            bucket: 0,
        });
    }

    let n_items = entries.len();
    let n_buckets = n_items;
    for entry in &mut entries {
        entry.bucket = (entry.hash as usize) % n_buckets;
    }

    let mut order: Vec<usize> = (0..n_items).collect();
    order.sort_by_key(|&raw| (entries[raw].bucket, entries[raw].full_path.as_str()));
    let mut raw_to_sorted = vec![0u32; n_items];
    for (sorted_idx, &raw_idx) in order.iter().enumerate() {
        raw_to_sorted[raw_idx] = sorted_idx as u32;
    }

    let mut children = vec![Vec::<u32>::new(); n_items];
    for (sorted_idx, &raw_idx) in order.iter().enumerate() {
        if let Some(parent_raw) = entries[raw_idx].parent_raw {
            children[parent_raw].push(sorted_idx as u32);
        }
    }

    let root_start: u32 = 24;
    let table_len = 8 + 4 * n_buckets + 24 * n_items;
    let root_end = root_start + table_len as u32;

    let mut buf = Vec::with_capacity(root_end as usize + icons.len() * 512);
    buf.extend_from_slice(b"GVariant");
    buf.extend_from_slice(&0u32.to_le_bytes());
    buf.extend_from_slice(&0u32.to_le_bytes());
    buf.extend_from_slice(&root_start.to_le_bytes());
    buf.extend_from_slice(&root_end.to_le_bytes());
    // Reserve hash table header + buckets + items.
    buf.resize(root_end as usize, 0);

    let mut item_meta = Vec::with_capacity(n_items);
    for &raw_idx in &order {
        let entry = &entries[raw_idx];
        let key_start = buf.len() as u32;
        buf.extend_from_slice(entry.key.as_bytes());
        let (value_start, value_end) = if entry.is_dir {
            align(&mut buf, 4);
            let start = buf.len() as u32;
            for &child in &children[raw_idx] {
                buf.extend_from_slice(&child.to_le_bytes());
            }
            (start, buf.len() as u32)
        } else {
            align(&mut buf, 8);
            let start = buf.len() as u32;
            buf.extend_from_slice(&(entry.svg.len() as u32).to_le_bytes());
            buf.extend_from_slice(&0u32.to_le_bytes());
            buf.extend_from_slice(entry.svg.as_bytes());
            buf.push(0);
            buf.push(0);
            buf.extend_from_slice(b"(uuay)");
            (start, buf.len() as u32)
        };
        item_meta.push((
            entry.hash,
            entry.parent_raw.map_or(u32::MAX, |p| raw_to_sorted[p]),
            key_start,
            entry.key.len() as u16,
            if entry.is_dir { b'L' } else { b'v' },
            value_start,
            value_end,
        ));
    }

    let mut table = Vec::with_capacity(table_len);
    table.extend_from_slice(&0u32.to_le_bytes()); // n_bloom_words = 0
    table.extend_from_slice(&(n_buckets as u32).to_le_bytes());
    let mut cursor = 0usize;
    for bucket in 0..n_buckets {
        table.extend_from_slice(&(cursor as u32).to_le_bytes());
        while cursor < n_items && entries[order[cursor]].bucket == bucket {
            cursor += 1;
        }
    }
    for (hash, parent, key_start, key_size, kind, value_start, value_end) in item_meta {
        table.extend_from_slice(&hash.to_le_bytes());
        table.extend_from_slice(&parent.to_le_bytes());
        table.extend_from_slice(&key_start.to_le_bytes());
        table.extend_from_slice(&key_size.to_le_bytes());
        table.push(kind);
        table.push(0);
        table.extend_from_slice(&value_start.to_le_bytes());
        table.extend_from_slice(&value_end.to_le_bytes());
    }
    buf[root_start as usize..root_end as usize].copy_from_slice(&table);
    buf
}

fn djb2(text: &str) -> u32 {
    text.bytes().fold(5381u32, |hash, byte| {
        hash.wrapping_mul(33).wrapping_add(byte as i8 as i32 as u32)
    })
}

fn align(buf: &mut Vec<u8>, alignment: usize) {
    let rem = buf.len() % alignment;
    if rem != 0 {
        buf.resize(buf.len() + (alignment - rem), 0);
    }
}
