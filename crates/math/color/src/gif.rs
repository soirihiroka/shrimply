use rayon::prelude::*;

use super::Color;

pub const GIF_ALPHA_THRESHOLD: u8 = 128;
const GIF_PALETTE_SIZE: usize = 256;
const GIF_OPAQUE_COLORS: usize = GIF_PALETTE_SIZE - 1;
const GIF_HISTOGRAM_BITS: usize = 5;
const GIF_HISTOGRAM_LEVELS: usize = 1 << GIF_HISTOGRAM_BITS;
const GIF_HISTOGRAM_SIZE: usize =
    GIF_HISTOGRAM_LEVELS * GIF_HISTOGRAM_LEVELS * GIF_HISTOGRAM_LEVELS;
const GIF_ERROR_DIVISOR: i32 = 16;

pub struct GifQuantization {
    pub indices: Vec<u8>,
    pub palette: [u32; GIF_PALETTE_SIZE],
}

#[derive(Clone, Copy, Default)]
struct HistogramEntry {
    color: Color<u8>,
    count: u64,
}

struct PaletteBox {
    entries: Vec<HistogramEntry>,
}

impl PaletteBox {
    fn range(&self, channel: fn(Color<u8>) -> u8) -> u8 {
        let minimum = self
            .entries
            .iter()
            .map(|entry| channel(entry.color))
            .min()
            .unwrap_or_default();
        let maximum = self
            .entries
            .iter()
            .map(|entry| channel(entry.color))
            .max()
            .unwrap_or_default();
        maximum - minimum
    }

    fn score(&self) -> u64 {
        let range = self
            .range(|color| color.r)
            .max(self.range(|color| color.g))
            .max(self.range(|color| color.b));
        u64::from(range) * self.entries.iter().map(|entry| entry.count).sum::<u64>()
    }

    fn split(mut self) -> Option<(Self, Self)> {
        if self.entries.len() < 2 {
            return None;
        }
        let channels: [fn(Color<u8>) -> u8; 3] =
            [|color| color.r, |color| color.g, |color| color.b];
        let channel = channels
            .into_iter()
            .max_by_key(|channel| self.range(*channel))
            .expect("GIF colors always have RGB channels");
        self.entries
            .sort_unstable_by_key(|entry| channel(entry.color));
        let halfway = self.entries.iter().map(|entry| entry.count).sum::<u64>() / 2;
        let mut accumulated = 0;
        let split = self
            .entries
            .iter()
            .enumerate()
            .find_map(|(index, entry)| {
                accumulated += entry.count;
                (accumulated >= halfway && index + 1 < self.entries.len()).then_some(index + 1)
            })
            .unwrap_or(self.entries.len() / 2);
        let other = self.entries.split_off(split);
        Some((self, Self { entries: other }))
    }

    fn average(&self) -> Color<u8> {
        let count = self.entries.iter().map(|entry| entry.count).sum::<u64>();
        let average = |channel: fn(Color<u8>) -> u8| {
            (self
                .entries
                .iter()
                .map(|entry| u64::from(channel(entry.color)) * entry.count)
                .sum::<u64>()
                / count) as u8
        };
        Color::<u8>::from_rgb(
            average(|color| color.r),
            average(|color| color.g),
            average(|color| color.b),
        )
    }
}

pub fn quantize_gif_rgba(rgba: &[u8], width: usize, height: usize) -> GifQuantization {
    assert_eq!(rgba.len(), width * height * 4, "GIF RGBA buffer size");
    let mut histogram = vec![HistogramEntry::default(); GIF_HISTOGRAM_SIZE];
    for pixel in rgba.as_chunks::<4>().0 {
        if pixel[3] < GIF_ALPHA_THRESHOLD {
            continue;
        }
        let entry = &mut histogram[histogram_index(pixel[0], pixel[1], pixel[2])];
        entry.color = Color::<u8>::from_rgb(pixel[0], pixel[1], pixel[2]);
        entry.count += 1;
    }
    let entries = histogram
        .into_iter()
        .filter(|entry| entry.count > 0)
        .collect::<Vec<_>>();
    let mut boxes = if entries.is_empty() {
        Vec::new()
    } else {
        vec![PaletteBox { entries }]
    };
    while boxes.len() < GIF_OPAQUE_COLORS {
        let Some(index) = boxes
            .iter()
            .enumerate()
            .filter(|(_, color_box)| color_box.entries.len() > 1)
            .max_by_key(|(_, color_box)| color_box.score())
            .map(|(index, _)| index)
        else {
            break;
        };
        let color_box = boxes.swap_remove(index);
        let Some((first, second)) = color_box.split() else {
            break;
        };
        boxes.push(first);
        boxes.push(second);
    }
    let mut colors = boxes.iter().map(PaletteBox::average).collect::<Vec<_>>();
    colors.sort_unstable_by_key(|color| (color.r, color.g, color.b));

    let mut palette = [u32::from_be_bytes([u8::MAX, 0, 0, 0]); GIF_PALETTE_SIZE];
    palette[0] = 0;
    for (entry, color) in palette.iter_mut().skip(1).zip(&colors) {
        *entry = u32::from_be_bytes([u8::MAX, color.r, color.g, color.b]);
    }

    let mut lookup = vec![0_u8; GIF_HISTOGRAM_SIZE];
    lookup
        .par_iter_mut()
        .enumerate()
        .for_each(|(histogram_index, index)| {
            let sample = histogram_color(histogram_index);
            *index = colors
                .iter()
                .enumerate()
                .min_by_key(|(_, color)| color_distance(sample, **color))
                .map_or(0, |(index, _)| (index + 1) as u8);
        });

    let mut indices = vec![0; width * height];
    let mut current_errors = vec![[0_i32; 3]; width + 2];
    let mut next_errors = vec![[0_i32; 3]; width + 2];
    for (row, output) in rgba
        .chunks_exact(width * 4)
        .zip(indices.chunks_exact_mut(width))
    {
        for (x, (pixel, output)) in row.as_chunks::<4>().0.iter().zip(output).enumerate() {
            if pixel[3] < GIF_ALPHA_THRESHOLD {
                current_errors[x + 1] = [0; 3];
                continue;
            }
            let adjusted = Color::<u8>::from_rgb(
                (i32::from(pixel[0]) + current_errors[x + 1][0] / GIF_ERROR_DIVISOR)
                    .clamp(0, i32::from(u8::MAX)) as u8,
                (i32::from(pixel[1]) + current_errors[x + 1][1] / GIF_ERROR_DIVISOR)
                    .clamp(0, i32::from(u8::MAX)) as u8,
                (i32::from(pixel[2]) + current_errors[x + 1][2] / GIF_ERROR_DIVISOR)
                    .clamp(0, i32::from(u8::MAX)) as u8,
            );
            let palette_index = lookup[histogram_index(adjusted.r, adjusted.g, adjusted.b)];
            *output = palette_index;
            let selected = colors[usize::from(palette_index) - 1];
            for (channel, (adjusted, selected)) in adjusted
                .to_array()
                .into_iter()
                .zip(selected.to_array())
                .take(3)
                .enumerate()
            {
                let error = i32::from(adjusted) - i32::from(selected);
                current_errors[x + 2][channel] += error * 7;
                next_errors[x][channel] += error * 3;
                next_errors[x + 1][channel] += error * 5;
                next_errors[x + 2][channel] += error;
            }
        }
        std::mem::swap(&mut current_errors, &mut next_errors);
        next_errors.fill([0; 3]);
    }
    GifQuantization { indices, palette }
}

fn histogram_index(red: u8, green: u8, blue: u8) -> usize {
    (usize::from(red) >> (8 - GIF_HISTOGRAM_BITS)) << (GIF_HISTOGRAM_BITS * 2)
        | (usize::from(green) >> (8 - GIF_HISTOGRAM_BITS)) << GIF_HISTOGRAM_BITS
        | (usize::from(blue) >> (8 - GIF_HISTOGRAM_BITS))
}

fn histogram_color(index: usize) -> Color<u8> {
    let mask = GIF_HISTOGRAM_LEVELS - 1;
    let expand = |value: usize| (value * usize::from(u8::MAX) / mask) as u8;
    Color::<u8>::from_rgb(
        expand(index >> (GIF_HISTOGRAM_BITS * 2) & mask),
        expand(index >> GIF_HISTOGRAM_BITS & mask),
        expand(index & mask),
    )
}

fn color_distance(first: Color<u8>, second: Color<u8>) -> u32 {
    let red = i32::from(first.r) - i32::from(second.r);
    let green = i32::from(first.g) - i32::from(second.g);
    let blue = i32::from(first.b) - i32::from(second.b);
    (red * red * 2 + green * green * 4 + blue * blue) as u32
}
