use super::*;

/// Deliberately overlapping bounds, long labels, and content beyond both viewport axes.
pub fn tracks(stress: bool) -> Vec<Track> {
    const NORMAL_TRACKS: usize = 16;
    const STRESS_TRACKS: usize = 128;
    const NORMAL_CLIPS: usize = 24;
    const STRESS_CLIPS: usize = 2048;
    (0..if stress { STRESS_TRACKS } else { NORMAL_TRACKS })
        .map(|row| {
            let (kind, fill, accent) = match row % 4 {
                0 => ("Comment", Color::PURPLE5, Color::PURPLE1),
                1 => ("Caption", Color::YELLOW5, Color::YELLOW1),
                2 => ("Video", Color::BLUE5, Color::BLUE1),
                _ => ("Audio", Color::GREEN5, Color::GREEN1),
            };
            Track {
                label: format!("{kind} {}", row / 4 + 1),
                fill,
                accent,
                selected: row == 2,
                clips: (0..if stress { STRESS_CLIPS } else { NORMAL_CLIPS })
                    .map(|index| {
                        let start = Time::from_fraction((index * 7 + row % 3) as i64, 2);
                        Clip {
                            start,
                            end: start.saturating_add(Time::from_seconds(if index % 3 == 0 {
                                5
                            } else {
                                3
                            })),
                            label: format!(
                                "{kind} {} — a long sample label for clipping and scrolling",
                                index + 1
                            ),
                            selected: row == 2 && index == 1,
                        }
                    })
                    .collect(),
            }
        })
        .collect()
}
