use crate::tables::SPECTRUM;

/// Standard ISO/IEC 13818-7 spectral Huffman codebooks (Table 4.4 / Annex A).
/// Pairs (A/B) share tuple width and range but model peaked vs. flatter distributions.
#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Codebook {
    #[default]
    Zero = 0,
    /// 4-tuples, signed values in [-1, 1].
    QuadSigned1A = 1,
    QuadSigned1B = 2,
    /// 4-tuples, unsigned magnitudes in [0, 2] followed by sign bits.
    QuadUnsigned2A = 3,
    QuadUnsigned2B = 4,
    /// 2-tuples, signed values in [-4, 4].
    PairSigned4A = 5,
    PairSigned4B = 6,
    /// 2-tuples, unsigned magnitudes in [0, 7] followed by sign bits.
    PairUnsigned7A = 7,
    PairUnsigned7B = 8,
    /// 2-tuples, unsigned magnitudes in [0, 12] followed by sign bits.
    PairUnsigned12A = 9,
    PairUnsigned12B = 10,
    /// 2-tuples, unsigned magnitudes in [0, 16] with variable-length escape for >= 16.
    PairEscape = 11,
}

pub(crate) const CODEBOOK_COUNT: usize = 12;
pub(crate) const ESCAPE_LIMIT: u16 = 16;

#[inline]
pub(crate) const fn code_len(code: u32) -> u16 {
    (code & 31) as u16
}

#[derive(Clone, Copy)]
pub(crate) struct BookSpec {
    pub width: usize,
    pub limit: u16,
    pub signed: bool,
}

const BOOK_SPECS: [BookSpec; CODEBOOK_COUNT] = [
    BookSpec {
        width: 0,
        limit: 0,
        signed: false,
    }, // Zero
    BookSpec {
        width: 4,
        limit: 1,
        signed: true,
    }, // QuadSigned1A
    BookSpec {
        width: 4,
        limit: 1,
        signed: true,
    }, // QuadSigned1B
    BookSpec {
        width: 4,
        limit: 2,
        signed: false,
    }, // QuadUnsigned2A
    BookSpec {
        width: 4,
        limit: 2,
        signed: false,
    }, // QuadUnsigned2B
    BookSpec {
        width: 2,
        limit: 4,
        signed: true,
    }, // PairSigned4A
    BookSpec {
        width: 2,
        limit: 4,
        signed: true,
    }, // PairSigned4B
    BookSpec {
        width: 2,
        limit: 7,
        signed: false,
    }, // PairUnsigned7A
    BookSpec {
        width: 2,
        limit: 7,
        signed: false,
    }, // PairUnsigned7B
    BookSpec {
        width: 2,
        limit: 12,
        signed: false,
    }, // PairUnsigned12A
    BookSpec {
        width: 2,
        limit: 12,
        signed: false,
    }, // PairUnsigned12B
    BookSpec {
        width: 2,
        limit: 16,
        signed: false,
    }, // PairEscape
];

pub(crate) const ACTIVE_BOOKS: [Codebook; 11] = [
    Codebook::QuadSigned1A,
    Codebook::QuadSigned1B,
    Codebook::QuadUnsigned2A,
    Codebook::QuadUnsigned2B,
    Codebook::PairSigned4A,
    Codebook::PairSigned4B,
    Codebook::PairUnsigned7A,
    Codebook::PairUnsigned7B,
    Codebook::PairUnsigned12A,
    Codebook::PairUnsigned12B,
    Codebook::PairEscape,
];

impl Codebook {
    #[inline]
    pub const fn id(self) -> u8 {
        self as u8
    }

    #[inline]
    pub const fn index(self) -> usize {
        self as usize
    }

    #[inline]
    pub const fn from_id(id: u8) -> Self {
        match id {
            0 => Self::Zero,
            1 => Self::QuadSigned1A,
            2 => Self::QuadSigned1B,
            3 => Self::QuadUnsigned2A,
            4 => Self::QuadUnsigned2B,
            5 => Self::PairSigned4A,
            6 => Self::PairSigned4B,
            7 => Self::PairUnsigned7A,
            8 => Self::PairUnsigned7B,
            9 => Self::PairUnsigned12A,
            10 => Self::PairUnsigned12B,
            11 => Self::PairEscape,
            _ => unreachable!(),
        }
    }

    #[inline]
    pub const fn spec(self) -> BookSpec {
        BOOK_SPECS[self as usize]
    }

    /// Smallest codebook whose non-escape range fits `peak`.
    #[inline]
    pub const fn first_for_peak(peak: u16) -> Self {
        match peak {
            0 => Self::Zero,
            1 => Self::QuadSigned1A,
            2 => Self::QuadUnsigned2A,
            3..=4 => Self::PairSigned4A,
            5..=7 => Self::PairUnsigned7A,
            8..=12 => Self::PairUnsigned12A,
            _ => Self::PairEscape,
        }
    }

    #[inline]
    pub fn tuple_index(self, values: &[i16]) -> usize {
        let spec = self.spec();
        let limit = usize::from(spec.limit);
        let radix = if spec.signed {
            2 * limit + 1
        } else {
            limit + 1
        };
        values.iter().fold(0, |index, &value| {
            let digit = if spec.signed {
                (value + spec.limit as i16) as usize
            } else {
                usize::from(value.unsigned_abs()).min(limit)
            };
            index * radix + digit
        })
    }

    pub fn write(self, out: &mut Bits, values: &[i16]) {
        let spec = self.spec();
        let table = SPECTRUM[self.index() - 1];
        for tuple in values.chunks_exact(spec.width) {
            out.code(table[self.tuple_index(tuple)]);
            if spec.signed {
                continue;
            }
            for &sample in tuple.iter().filter(|&&sample| sample != 0) {
                out.put(1, u32::from(sample < 0));
            }
            if self == Self::PairEscape {
                for sample in tuple
                    .iter()
                    .map(|sample| u32::from(sample.unsigned_abs()))
                    .filter(|&sample| sample >= u32::from(ESCAPE_LIMIT))
                {
                    let width = sample.ilog2();
                    out.put((width - 3) as usize, (1 << (width - 3)) - 2);
                    out.put(width as usize, sample - (1 << width));
                }
            }
        }
    }
}

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
        self.put(usize::from(code_len(code)), code >> 5);
    }
}

#[inline]
pub(crate) fn escape_bits(magnitude: u16) -> u16 {
    if magnitude >= ESCAPE_LIMIT {
        (2 * magnitude.ilog2() - 3) as u16
    } else {
        0
    }
}

/// Compute exact Huffman bit cost for relevant codebooks in one pass,
/// reusing paired tuple indices across (A, B) codebook pairs.
/// Stops one tier above `first` since wider codebooks rarely pay back section overhead.
pub(crate) fn codebook_costs(values: &[i16], peak: u16) -> [u16; CODEBOOK_COUNT] {
    let mut costs = [u16::MAX; CODEBOOK_COUNT];
    if peak == 0 {
        costs[Codebook::Zero.index()] = 0;
        return costs;
    }
    let first = Codebook::first_for_peak(peak).id();
    let max_id = (first + 3).min(Codebook::PairEscape.id());
    let signs = values.iter().filter(|&&sample| sample != 0).count() as u16;
    let end_pair = max_id.min(Codebook::PairUnsigned12A.id());
    for id in (first..=end_pair).step_by(2) {
        let book = Codebook::from_id(id);
        let spec = book.spec();
        let table = usize::from(id - 1);
        let initial = if spec.signed { 0 } else { signs };
        let mut c0 = initial;
        let mut c1 = initial;
        for tuple in values.chunks_exact(spec.width) {
            let index = book.tuple_index(tuple);
            c0 += code_len(SPECTRUM[table][index]);
            c1 += code_len(SPECTRUM[table + 1][index]);
        }
        costs[usize::from(id)] = c0;
        costs[usize::from(id + 1)] = c1;
    }
    if max_id == Codebook::PairEscape.id() {
        let esc = Codebook::PairEscape;
        let mut c11 = signs;
        for tuple in values.chunks_exact(esc.spec().width) {
            c11 += code_len(SPECTRUM[esc.index() - 1][esc.tuple_index(tuple)]);
        }
        for &sample in values {
            c11 += escape_bits(sample.unsigned_abs());
        }
        costs[esc.index()] = c11;
    }
    costs
}
