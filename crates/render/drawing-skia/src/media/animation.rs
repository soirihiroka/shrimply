//! Source-frame timing shared by animated raster formats and all renderers.
use super::{gif::Gif, jxl};
use shrimply_math_core::{Fraction, Time};
use skia_safe::Image;

pub(super) enum Decoder {
    Gif(Gif),
    JpegXl(Box<jxl::JpegXl>),
}

pub struct Animation {
    decoder: Decoder,
    starts: Vec<Time>,
    pub duration: Time,
    last: Option<(usize, Image)>,
}

impl Animation {
    pub fn probe(bytes: &[u8]) -> Result<Option<Self>, String> {
        if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
            return Gif::open(bytes).map(Some);
        }
        if jxl::recognizes(bytes) {
            return jxl::JpegXl::probe_animation(bytes);
        }
        Ok(None)
    }

    pub fn open(bytes: &[u8]) -> Result<Self, String> {
        Self::probe(bytes)?.ok_or_else(|| "Expected an animated image".into())
    }

    pub(super) fn new(decoder: Decoder, mut delays: Vec<Time>) -> Result<Self, String> {
        let mut duration = Time::ZERO;
        for delay in &mut delays {
            let start = duration;
            duration = duration.saturating_add(*delay);
            *delay = start;
        }
        if duration == Time::ZERO {
            return Err("Animated image has no frame with a positive duration".into());
        }
        Ok(Self {
            decoder,
            duration,
            starts: delays,
            last: None,
        })
    }

    pub fn is_gif(&self) -> bool {
        matches!(self.decoder, Decoder::Gif(_))
    }

    pub fn dimensions(&self) -> [u32; 2] {
        match &self.decoder {
            Decoder::Gif(gif) => gif.dimensions(),
            Decoder::JpegXl(jxl) => jxl.dimensions(),
        }
    }

    pub fn average_frame_rate(&self) -> Fraction {
        Fraction::from(self.starts.len() as u64) / self.duration.seconds
    }

    pub fn frame_at(&mut self, time: Time) -> Result<(Time, Image), String> {
        let index = self
            .starts
            .partition_point(|start| *start <= time)
            .saturating_sub(1);
        if let Some((last, image)) = &self.last
            && *last == index
        {
            return Ok((self.starts[index], image.clone()));
        }
        let image = match &mut self.decoder {
            Decoder::Gif(gif) => gif.frame(index, self.last.as_ref())?,
            Decoder::JpegXl(jxl) => jxl.frame(index)?,
        };
        self.last = Some((index, image.clone()));
        Ok((self.starts[index], image))
    }
}
