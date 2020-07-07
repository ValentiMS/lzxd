/// > An LZXD bitstream is encoded as a sequence of aligned 16-bit integers stored in the
/// > least-significant- byte to most-significant-byte order, also known as byte-swapped,
/// > or little-endian, words. Given an input stream of bits named a, b, c,..., x, y, z,
/// > A, B, C, D, E, F, the output byte stream MUST be as [ 0| 1| 2| 3|...|30|31].
///
/// It is worth mentioning that older revisions of the document explain this better:
///
/// > Given an input stream of bits named a, b, c, ..., x, y, z, A, B, C, D, E, F, the output
/// > byte stream (with byte boundaries highlighted) would be as follows:
/// > [i|j|k|l|m|n|o#p|a|b|c|d|e|f|g|h#y|z|A|B|C|D|E|F#q|r|s|t|u|v|w|x]
pub struct Bitstream<'a> {
    buffer: &'a [u8],
    // Next number in the bitstream.
    n: u16,
    // How many bits left in the current `n`.
    remaining: u8,
}

impl<'a> Bitstream<'a> {
    pub fn new(buffer: &'a [u8]) -> Self {
        Self {
            buffer,
            n: 0,
            remaining: 0,
        }
    }

    // Advance the buffer to the next 16-bit integer.
    #[inline(always)]
    fn advance_buffer(&mut self) {
        self.remaining = 16;
        self.n = u16::from_le_bytes([self.buffer[0], self.buffer[1]]);
        self.buffer = &self.buffer[2..];
    }

    pub fn read_bit(&mut self) -> u16 {
        if self.remaining == 0 {
            self.advance_buffer();
        }

        self.remaining -= 1;
        self.n = self.n.rotate_left(1);
        self.n & 1
    }

    pub fn read_bits(&mut self, bits: u8) -> u16 {
        assert!(bits <= 16);
        debug_assert!(self.remaining <= 16);

        if bits <= self.remaining {
            self.remaining -= bits;
            self.n = self.n.rotate_left(bits as u32);
            self.n & ((1 << bits) - 1)
        } else {
            // No need to store `rol` result in `n` as we're about to overwrite it.
            let hi = self.n.rotate_left(self.remaining as u32) & ((1 << self.remaining) - 1);
            let bits = bits - self.remaining;
            self.advance_buffer();

            self.remaining -= bits;
            self.n = self.n.rotate_left(bits as u32);
            // `bits` may be 16 which would overflow the left shift, operate on `u32` and trunc.
            let lo = self.n & ((1u32 << bits) as u16).wrapping_sub(1);

            ((hi as u32) << bits) as u16 | lo
        }
    }

    pub fn peek_bits(&self, bits: u8) -> u16 {
        // Copy paste of `read_bits`, but without advancing the buffer.
        assert!(bits <= 16);

        if bits <= self.remaining {
            self.n.rotate_left(bits as u32) & ((1 << bits) - 1)
        } else {
            let hi = self.n.rotate_left(self.remaining as u32) & ((1 << self.remaining) - 1);
            let bits = bits - self.remaining;

            // We may peek more than we need (i.e. at the end of a chunk), due to the way
            // our decoder is implemented. This is a bit ugly but luckily we can pretend
            // there are just zeros after.
            let n = if self.buffer.is_empty() {
                0
            } else {
                u16::from_le_bytes([self.buffer[0], self.buffer[1]])
            };
            let lo = n.rotate_left(bits as u32) & ((1u32 << bits) as u16).wrapping_sub(1);

            ((hi as u32) << bits) as u16 | lo
        }
    }

    pub fn read_u16_le(&mut self) -> u16 {
        self.read_bits(16).swap_bytes()
    }

    pub fn read_u32_le(&mut self) -> u32 {
        let lo = self.read_u16_le() as u32;
        let hi = self.read_u16_le() as u32;
        (hi << 16) | lo
    }

    pub fn read_u24_be(&mut self) -> u32 {
        let hi = self.read_bits(16) as u32;
        let lo = self.read_bits(8) as u32;
        hi << 8 | lo
    }

    pub fn is_empty(&self) -> bool {
        // > the output bitstream is padded with up to 15 bits of zeros to realign the bitstream
        // > on a 16-bit boundary (even byte boundary) for the next 32 KB of data.
        //
        // TODO but how likely it is to have valid 0 data in the last 15 bits? we would
        // misinterpret this unless we know the decompressed chunk length to know when to
        // stop reading
        self.buffer.is_empty() && self.peek_bits(self.remaining) == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_sequential() {
        // 0..=10 and padding using the least amount of bits possible, read LTR
        let ns = [0b0_1_10_11_100_101_110_1u16, 0b11_1000_1001_1010_00u16];
        let bit_lengths = [1u8, 1, 2, 2, 3, 3, 3, 3, 4, 4, 4];

        // Convert input sequence of 16-bit integers to byte-stream
        let mut bytes = Vec::with_capacity(ns.len() * 2);
        ns.iter().for_each(|n| bytes.extend(&n.to_le_bytes()));

        let mut bitstream = Bitstream::new(&bytes);
        bit_lengths
            .iter()
            .copied()
            .enumerate()
            .for_each(|(value, bit_length)| {
                assert_eq!(bitstream.read_bits(bit_length), value as u16);
            });
    }

    #[test]
    fn read_16le_aligned() {
        let ns = [0b11100000_00000111_u16, 0b00011111_11111000];
        let mut bytes = Vec::with_capacity(ns.len() * 2);
        ns.iter().for_each(|n| bytes.extend(&n.to_le_bytes()));

        let mut bitstream = Bitstream::new(&bytes);
        assert_eq!(bitstream.read_u16_le(), 0b00000111_11100000);
        assert_eq!(bitstream.read_u16_le(), 0b11111000_00011111);
    }

    #[test]
    fn read_16le_unaligned() {
        let ns = [0b00000000000_10001u16, 0b10000000001_00000];
        let mut bytes = Vec::with_capacity(ns.len() * 2);
        ns.iter().for_each(|n| bytes.extend(&n.to_le_bytes()));

        let mut bitstream = Bitstream::new(&bytes);

        assert_eq!(bitstream.read_bits(11), 0);
        assert_eq!(bitstream.read_u16_le(), 0b00000001_10001_100);
        assert_eq!(bitstream.read_bits(5), 0);
    }

    #[test]
    fn read_32le() {
        let bytes = [0x56, 0x78, 0x12, 0x34];
        let mut bitstream = Bitstream::new(&bytes);

        assert_eq!(bitstream.read_u32_le(), 0x12345678);
    }

    #[test]
    fn read_24be() {
        let ns = [0b0000_1100_0001_1000_u16, 0b0001_1000_0011_0000_u16];
        let mut bytes = Vec::with_capacity(ns.len() * 2);
        ns.iter().for_each(|n| bytes.extend(&n.to_le_bytes()));

        let mut bitstream = Bitstream::new(&bytes);

        assert_eq!(bitstream.read_bits(4), 0);
        assert_eq!(bitstream.read_u24_be(), 0b1100_0001_1000_0001_1000_0011);
        assert_eq!(bitstream.read_bits(4), 0);
    }

    #[test]
    fn is_empty() {
        let bytes = [];
        let bitstream = Bitstream::new(&bytes);
        assert!(bitstream.is_empty());

        let bytes = [0xab, 0xcd];
        let mut bitstream = Bitstream::new(&bytes);
        assert!(!bitstream.is_empty());
        bitstream.read_bits(15);
        assert!(!bitstream.is_empty());
        bitstream.read_bit();
        assert!(bitstream.is_empty());
    }

    #[test]
    fn check_read_bit() {
        let bytes = [0b0110_1001, 0b1001_0110];
        let mut bitstream_1 = Bitstream::new(&bytes);
        let mut bitstream_n = Bitstream::new(&bytes);

        (0..16).for_each(|_| assert_eq!(bitstream_1.read_bit(), bitstream_n.read_bits(1)));
    }
}
