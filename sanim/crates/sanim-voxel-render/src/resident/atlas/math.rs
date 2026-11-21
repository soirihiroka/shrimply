//! Linear-light, alpha-weighted area reduction includes every odd edge texel.
use shrimply_math_color::Color;
use std::sync::OnceLock;

pub(super) fn next_size(dimensions: [u32; 2]) -> [u32; 2] {
    dimensions.map(|size| (size / 2).max(1))
}

pub(super) fn downsample(dimensions: [u32; 2], pixels: &[u8]) -> ([u32; 2], Vec<u8>) {
    static LINEAR: OnceLock<[f64; 256]> = OnceLock::new();
    let linear = LINEAR.get_or_init(|| {
        std::array::from_fn(|value| f64::from(Color::<u8>::from_gray(value as u8).to_linear().r))
    });
    let next = next_size(dimensions);
    let mut output = vec![0; next[0] as usize * next[1] as usize * 4];
    let scale = [
        f64::from(dimensions[0]) / f64::from(next[0]),
        f64::from(dimensions[1]) / f64::from(next[1]),
    ];
    for y in 0..next[1] {
        for x in 0..next[0] {
            let start = [f64::from(x) * scale[0], f64::from(y) * scale[1]];
            let end = [f64::from(x + 1) * scale[0], f64::from(y + 1) * scale[1]];
            let mut sum = [0.; 4];
            for sy in start[1].floor() as u32..(end[1].ceil() as u32).min(dimensions[1]) {
                let weight_y =
                    (end[1].min(f64::from(sy + 1)) - start[1].max(f64::from(sy))).max(0.);
                for sx in start[0].floor() as u32..(end[0].ceil() as u32).min(dimensions[0]) {
                    let weight = weight_y
                        * (end[0].min(f64::from(sx + 1)) - start[0].max(f64::from(sx))).max(0.);
                    let at = (sx as usize + dimensions[0] as usize * sy as usize) * 4;
                    let alpha = f64::from(pixels[at + 3]) / 255.;
                    for channel in 0..3 {
                        sum[channel] += linear[pixels[at + channel] as usize] * alpha * weight;
                    }
                    sum[3] += alpha * weight;
                }
            }
            if sum[3] > 0. {
                let color = Color::<f32>::new(
                    (sum[0] / sum[3]) as f32,
                    (sum[1] / sum[3]) as f32,
                    (sum[2] / sum[3]) as f32,
                    (sum[3] / (scale[0] * scale[1])) as f32,
                )
                .linear_to_srgb::<true>();
                let rgba = Color::<u8>::from_srgba(color.to_array()).to_array();
                let at = (x as usize + next[0] as usize * y as usize) * 4;
                output[at..at + 4].copy_from_slice(&rgba);
            }
        }
    }
    (next, output)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn averaging_is_linear_light_and_preserves_visible_color_across_cutouts() {
        let (_, gray) = downsample([2, 1], &[0, 0, 0, 255, 255, 255, 255, 255]);
        assert_eq!(gray, [188, 188, 188, 255]);
        let (_, blue) = downsample([2, 1], &[0, 100, 255, 255, 255, 0, 0, 0]);
        assert_eq!(blue, [0, 100, 255, 128]);
        let (_, transparent) = downsample([2, 1], &[255, 0, 0, 0, 0, 255, 0, 0]);
        assert_eq!(transparent, [0; 4]);
    }
    #[test]
    fn odd_edges_and_single_axes_contribute_to_the_last_level() {
        let (size, pixel) = downsample([3, 1], &[0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255, 255]);
        assert_eq!(size, [1, 1]);
        assert_eq!(pixel, [156, 156, 156, 255]);
        let (_, horizontal) =
            downsample([3, 1], &[20, 40, 60, 255, 20, 40, 60, 255, 20, 40, 60, 255]);
        let (_, vertical) =
            downsample([1, 3], &[20, 40, 60, 255, 20, 40, 60, 255, 20, 40, 60, 255]);
        assert_eq!(horizontal, vertical);
        assert_eq!(horizontal, [20, 40, 60, 255]);
    }
}
