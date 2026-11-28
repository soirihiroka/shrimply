use crate::tables::SPECTRUM;

#[derive(Default)]
pub(crate) struct Bits {
    pub bytes: Vec<u8>,
    pub len: usize,
}

impl Bits {
    pub fn put(&mut self, width: usize, value: u32) {
        debug_assert!(width == 32 || value < 1 << width);
        let mut left = width;
        while left != 0 {
            let offset = self.len % 8;
            if offset == 0 {
                self.bytes.push(0);
            }
            let written = left.min(8 - offset);
            left -= written;
            let mask = (1 << written) - 1;
            *self.bytes.last_mut().unwrap() |=
                (((value >> left) & mask) as u8) << (8 - offset - written);
            self.len += written;
        }
    }

    pub fn code(&mut self, code: u32) {
        self.put((code & 31) as usize, code >> 5);
    }
}

pub(crate) struct Book {
    pub width: usize,
    limit: u16,
    signed: bool,
}

impl Book {
    pub fn new(id: u8) -> Self {
        Self {
            width: if id <= 4 { 4 } else { 2 },
            limit: match id {
                1 | 2 => 1,
                3 | 4 => 2,
                5 | 6 => 4,
                7 | 8 => 7,
                9 | 10 => 12,
                11 => 16,
                _ => unreachable!(),
            },
            signed: matches!(id, 1 | 2 | 5 | 6),
        }
    }

    pub fn code(&self, id: u8, values: &[i16]) -> u32 {
        let limit = self.limit as usize;
        let radix = if self.signed {
            2 * limit + 1
        } else {
            limit + 1
        };
        let index = values.iter().fold(0, |index, &value| {
            let digit = if self.signed {
                (value + self.limit as i16) as usize
            } else {
                usize::from(value.unsigned_abs()).min(limit)
            };
            index * radix + digit
        });
        SPECTRUM[usize::from(id - 1)][index]
    }

    pub fn cost(&self, id: u8, values: &[i16]) -> u16 {
        let mut bits = 0;
        for tuple in values.chunks_exact(self.width) {
            bits += (self.code(id, tuple) & 31) as u16;
        }
        if !self.signed {
            for &sample in values {
                bits += u16::from(sample != 0);
                if id == 11 && sample.unsigned_abs() >= 16 {
                    bits += (2 * sample.unsigned_abs().ilog2() - 3) as u16;
                }
            }
        }
        bits
    }

    pub fn write(&self, out: &mut Bits, id: u8, values: &[i16]) {
        for tuple in values.chunks_exact(self.width) {
            out.code(self.code(id, tuple));
            if self.signed {
                continue;
            }
            for &sample in tuple.iter().filter(|&&sample| sample != 0) {
                out.put(1, u32::from(sample < 0));
            }
            if id == 11 {
                for sample in tuple
                    .iter()
                    .map(|sample| u32::from(sample.unsigned_abs()))
                    .filter(|&sample| sample >= 16)
                {
                    let width = sample.ilog2();
                    out.put((width - 3) as usize, (1 << (width - 3)) - 2);
                    out.put(width as usize, sample - (1 << width));
                }
            }
        }
    }
}

pub(crate) fn best_book(values: &[i16], peak: u16) -> (u8, u16) {
    if peak == 0 {
        return (0, 0);
    }
    let first = match peak {
        1 => 1,
        2 => 3,
        3..=4 => 5,
        5..=7 => 7,
        8..=12 => 9,
        _ => 11,
    };
    (first..=11)
        .map(|id| (id, Book::new(id).cost(id, values)))
        .min_by_key(|&(_, cost)| cost)
        .unwrap()
}
