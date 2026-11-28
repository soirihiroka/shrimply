//! Standalone ADTS transport for comparison against other encoders.
use std::{
    env,
    fs::File,
    io::{self, BufWriter, Write},
};

use shrimply_aac::{Encoder, FRAME_SAMPLES};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let [input, output, rate, bitrate] = env::args()
        .skip(1)
        .collect::<Vec<_>>()
        .try_into()
        .map_err(|_| "Expected input.f32 output.aac sample-rate bit-rate")?;
    let mut encoder = Encoder::new(rate.parse()?, bitrate.parse()?)?;
    let rate_index = (encoder.configuration()[0] & 7) << 1 | encoder.configuration()[1] >> 7;
    let input = std::fs::read(input)?;
    if input.is_empty() || input.len() % 8 != 0 {
        return Err("Expected nonempty stereo f32le PCM".into());
    }
    let mut output = BufWriter::new(File::create(output)?);
    for pcm in input.chunks(FRAME_SAMPLES * 8).chain([&[][..]; 2]) {
        let mut block = [[0.0; 2]; FRAME_SAMPLES];
        for (frame, bytes) in block.iter_mut().zip(pcm.as_chunks::<8>().0) {
            *frame = std::array::from_fn(|channel| {
                let offset = channel * 4;
                f32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
            });
        }
        if let Some(packet) = encoder.encode(&block) {
            write_adts(&mut output, rate_index, &packet)?;
        }
    }
    output.flush()?;
    Ok(())
}

fn write_adts(output: &mut impl Write, rate_index: u8, packet: &[u8]) -> io::Result<()> {
    let length = packet.len() + 7;
    output.write_all(&[
        0xff,
        0xf1,
        0x40 | (rate_index << 2),    // AAC-LC.
        0x80 | (length >> 11) as u8, // Stereo.
        (length >> 3) as u8,
        ((length & 7) as u8) << 5 | 0x1f,
        0xfc,
    ])?;
    output.write_all(packet)
}
