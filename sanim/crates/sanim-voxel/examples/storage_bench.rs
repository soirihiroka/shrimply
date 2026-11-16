use sanim_voxel::Volume;
use std::{hint::black_box, time::Instant};

fn main() {
    const EDITS: usize = 10_000;
    let volume = Volume::new([512; 3])
        .unwrap()
        .fill_color([0; 3], [512, 8, 512], 0x3584e4ff)
        .unwrap();
    let before = volume.resident_bytes();
    let start = Instant::now();
    let mut changed = 0;
    let mut current = volume.clone();
    for index in 0..EDITS {
        let value = u32::from(index % 2 == 0);
        let next = current.set([256, 0, 256], value).unwrap();
        changed += next
            .visit_changes(&current, |_, chunk| {
                black_box(chunk);
            })
            .unwrap();
        current = black_box(next);
    }
    let elapsed = start.elapsed();
    println!(
        "{EDITS} edits + persistent diffs: {elapsed:?}; {:.3} us/edit; changed_bricks={changed}; resident_before={before}; resident_after={}; chunks={}",
        elapsed.as_secs_f64() * 1_000_000.0 / EDITS as f64,
        current.resident_bytes(),
        current.chunk_count()
    );
    assert_eq!(changed, EDITS - 1);
    assert!(current.resident_bytes() <= before + 4096);
    let clone_start = Instant::now();
    for _ in 0..EDITS {
        black_box(current.clone());
    }
    println!("{EDITS} snapshot clones: {:?}", clone_start.elapsed());
}
