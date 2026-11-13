use sanim_core::{Overflow, math::scroll::bars};

#[test]
fn automatic_bars_follow_each_axis_and_hidden_never_paints() {
    let viewport = [0., 0., 100., 60.];
    assert!(bars(viewport, [0., 0.], [0., 0.], [Overflow::Auto; 2]).is_empty());
    assert!(bars(viewport, [80., 40.], [0., 0.], [Overflow::Hidden; 2]).is_empty());
    let vertical = bars(
        viewport,
        [80., 40.],
        [0., 0.],
        [Overflow::Hidden, Overflow::Auto],
    );
    assert_eq!(vertical.len(), 1);
    assert!(
        vertical[0].track[2] - vertical[0].track[0] < vertical[0].track[3] - vertical[0].track[1]
    );
    assert_eq!(
        bars(viewport, [80., 40.], [0., 0.], [Overflow::Auto; 2]).len(),
        2
    );
}

#[test]
fn thumb_matches_scroll_endpoints_and_resizes_with_content() {
    let viewport = [0., 0., 100., 200.];
    let start = bars(
        viewport,
        [0., 400.],
        [0., 0.],
        [Overflow::Hidden, Overflow::Scroll],
    );
    let end = bars(
        viewport,
        [0., 400.],
        [0., 400.],
        [Overflow::Hidden, Overflow::Scroll],
    );
    assert_eq!(start[0].thumb[1], start[0].track[1]);
    assert_eq!(end[0].thumb[3], end[0].track[3]);
    let short = bars(
        viewport,
        [0., 20.],
        [0., 20.],
        [Overflow::Hidden, Overflow::Scroll],
    );
    assert!(short[0].thumb[3] - short[0].thumb[1] > end[0].thumb[3] - end[0].thumb[1]);
    let fits = bars(
        viewport,
        [0., 0.],
        [0., 0.],
        [Overflow::Hidden, Overflow::Scroll],
    );
    assert_eq!(fits[0].thumb, fits[0].track);
}

#[test]
fn idle_overlay_keeps_adwaita_spacing_so_rounded_caps_are_not_cropped() {
    let viewport = [0., 0., 100., 200.];
    let result = bars(
        viewport,
        [0., 400.],
        [0., 0.],
        [Overflow::Hidden, Overflow::Scroll],
    );
    let bar = &result[0];
    assert_eq!(bar.thumb[2] - bar.thumb[0], 2.25, "3 CSS px at 96 DPI");
    assert_eq!(
        bar.thumb[1], 6.75,
        "9 CSS px leaves room at rounded viewport corners"
    );
    assert_eq!(viewport[2] - bar.thumb[2], 3., "4 CSS px cross-axis inset");
    let tail = bars(
        viewport,
        [0., 400.],
        [0., 400.],
        [Overflow::Hidden, Overflow::Scroll],
    );
    assert_eq!(viewport[3] - tail[0].thumb[3], 6.75);
    for tiny in [[0., 0., 5., 5.], [0., 0., 100., 10.]] {
        for bar in bars(tiny, [100., 100.], [0., 0.], [Overflow::Scroll; 2]) {
            for axis in 0..2 {
                assert!(bar.thumb[axis] >= tiny[axis]);
                assert!(bar.thumb[axis + 2] <= tiny[axis + 2]);
                assert!(bar.thumb[axis] < bar.thumb[axis + 2]);
            }
        }
    }
}
