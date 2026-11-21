use crate::project::{AudioItem, CaptionItem, CommentItem, Time, VideoItem};

pub trait TimeSlice {
    fn start(&self) -> Time;
    fn end(&self) -> Time;

    fn duration_nanos(&self) -> u64 {
        self.end()
            .as_nonnegative_nanos()
            .saturating_sub(self.start().as_nonnegative_nanos())
    }
}

impl TimeSlice for CommentItem {
    fn start(&self) -> Time {
        self.start
    }

    fn end(&self) -> Time {
        self.end
    }
}

impl TimeSlice for CaptionItem {
    fn start(&self) -> Time {
        self.start
    }

    fn end(&self) -> Time {
        self.end
    }
}

impl TimeSlice for VideoItem {
    fn start(&self) -> Time {
        self.start
    }

    fn end(&self) -> Time {
        self.end
    }
}

impl TimeSlice for AudioItem {
    fn start(&self) -> Time {
        self.start
    }

    fn end(&self) -> Time {
        self.end
    }
}

pub fn overlapping<'a, T: TimeSlice>(
    items: &'a [T],
    start: Time,
    end: Time,
) -> impl Iterator<Item = (usize, &'a T)> + 'a {
    let range_start = start.min(end);
    let range_end = start.max(end);
    let last = items.partition_point(|item| item.start() <= range_end);

    items[..last]
        .iter()
        .enumerate()
        .filter(move |(_, item)| item.end() >= range_start)
}

/// Closed-boundary lookup for validated, sorted, non-overlapping tracks.
/// Unlike `overlapping`, this also binary-searches the monotonically ordered ends.
pub fn overlapping_disjoint<'a, T: TimeSlice>(
    items: &'a [T],
    start: Time,
    end: Time,
) -> impl Iterator<Item = (usize, &'a T)> + 'a {
    let range_start = start.min(end);
    let range_end = start.max(end);
    let last = items.partition_point(|item| item.start() <= range_end);
    let first = items[..last].partition_point(|item| item.end() < range_start);
    items[first..last]
        .iter()
        .enumerate()
        .map(move |(index, item)| (first + index, item))
}

pub fn collides<T: TimeSlice>(items: &[T], start: Time, end: Time) -> bool {
    let range_start = start.min(end);
    let range_end = start.max(end);
    overlapping(items, range_start, range_end)
        .any(|(_, item)| item.start() < range_end && item.end() > range_start)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy)]
    struct Slice {
        start: Time,
        end: Time,
    }

    impl Slice {
        fn new(start: i64, end: i64) -> Self {
            Self {
                start: Time::from_seconds(start),
                end: Time::from_seconds(end),
            }
        }
    }

    impl TimeSlice for Slice {
        fn start(&self) -> Time {
            self.start
        }

        fn end(&self) -> Time {
            self.end
        }
    }

    #[test]
    fn finds_overlapping_items_in_sorted_track() {
        let items = [Slice::new(0, 2), Slice::new(3, 5), Slice::new(6, 8)];
        let indices: Vec<_> = overlapping(&items, Time::from_seconds(4), Time::from_seconds(7))
            .map(|(index, _)| index)
            .collect();

        assert_eq!(indices, [1, 2]);
    }

    #[test]
    fn accepts_reversed_bounds() {
        let items = [Slice::new(0, 2), Slice::new(3, 5), Slice::new(6, 8)];
        let indices: Vec<_> = overlapping(&items, Time::from_seconds(7), Time::from_seconds(4))
            .map(|(index, _)| index)
            .collect();

        assert_eq!(indices, [1, 2]);
    }

    #[test]
    fn keeps_item_that_starts_before_range_and_extends_into_it() {
        let items = [Slice::new(0, 10), Slice::new(2, 3), Slice::new(12, 14)];
        let indices: Vec<_> = overlapping(&items, Time::from_seconds(8), Time::from_seconds(9))
            .map(|(index, _)| index)
            .collect();

        assert_eq!(indices, [0]);
    }

    #[test]
    fn ignores_items_starting_after_range() {
        let items = [Slice::new(0, 1), Slice::new(5, 6), Slice::new(9, 10)];
        let indices: Vec<_> = overlapping(&items, Time::from_seconds(2), Time::from_seconds(4))
            .map(|(index, _)| index)
            .collect();

        assert!(indices.is_empty());
    }

    #[test]
    fn detects_real_collision() {
        let items = [Slice::new(0, 2), Slice::new(4, 6)];

        assert!(collides(
            &items,
            Time::from_seconds(1),
            Time::from_seconds(3)
        ));
    }

    #[test]
    fn allows_touching_edges() {
        let items = [Slice::new(0, 2), Slice::new(4, 6)];

        assert!(!collides(
            &items,
            Time::from_seconds(2),
            Time::from_seconds(4)
        ));
    }

    #[test]
    fn disjoint_lookup_preserves_boundaries_indices_and_reversed_bounds() {
        let items = [Slice::new(0, 2), Slice::new(3, 5), Slice::new(6, 8)];
        for (start, end) in [(2, 6), (6, 2), (0, 0), (8, 9), (9, 10)] {
            let start = Time::from_seconds(start);
            let end = Time::from_seconds(end);
            assert_eq!(
                overlapping_disjoint(&items, start, end)
                    .map(|(index, _)| index)
                    .collect::<Vec<_>>(),
                overlapping(&items, start, end)
                    .map(|(index, _)| index)
                    .collect::<Vec<_>>()
            );
        }
        assert_eq!(
            overlapping_disjoint::<Slice>(&[], Time::ZERO, Time::ZERO).count(),
            0
        );
    }

    #[test]
    fn disjoint_lookup_cost_is_logarithmic_far_into_a_large_track() {
        use std::cell::Cell;
        struct Measured<'a> {
            index: i64,
            comparisons: &'a Cell<usize>,
        }
        impl TimeSlice for Measured<'_> {
            fn start(&self) -> Time {
                self.comparisons.set(self.comparisons.get() + 1);
                Time::from_seconds(self.index * 2)
            }
            fn end(&self) -> Time {
                self.comparisons.set(self.comparisons.get() + 1);
                Time::from_seconds(self.index * 2 + 1)
            }
        }
        const ITEMS: i64 = 65_536;
        let comparisons = Cell::new(0);
        let items = (0..ITEMS)
            .map(|index| Measured {
                index,
                comparisons: &comparisons,
            })
            .collect::<Vec<_>>();
        let found = overlapping_disjoint(
            &items,
            Time::from_seconds(ITEMS),
            Time::from_seconds(ITEMS + 1),
        )
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
        assert_eq!(found, [ITEMS as usize / 2]);
        assert!(
            comparisons.get() <= 40,
            "{} comparisons for {ITEMS} items",
            comparisons.get()
        );
    }

    #[test]
    #[ignore = "manual benchmark: make android-project-bench"]
    fn disjoint_viewport_lookup_benchmark() {
        const ITEMS: i64 = 65_536;
        const LOOKUPS: usize = 256;
        let items = (0..ITEMS)
            .map(|index| Slice::new(index * 2, index * 2 + 1))
            .collect::<Vec<_>>();
        let start = Time::from_seconds((ITEMS - 2) * 2);
        let end = Time::from_seconds(ITEMS * 2);
        let baseline = std::time::Instant::now();
        let mut previous = 0;
        for _ in 0..LOOKUPS {
            previous += std::hint::black_box(overlapping(&items, start, end).count());
        }
        let baseline = baseline.elapsed();
        let indexed = std::time::Instant::now();
        let mut current = 0;
        for _ in 0..LOOKUPS {
            current += std::hint::black_box(overlapping_disjoint(&items, start, end).count());
        }
        let indexed = indexed.elapsed();
        assert_eq!(previous, current);
        assert_eq!(current, LOOKUPS * 2);
        eprintln!(
            "Viewport lookup: {ITEMS} clips, {LOOKUPS} lookups; general scan {baseline:?}, sorted-disjoint lookup {indexed:?}"
        );
    }
}
