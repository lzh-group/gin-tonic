use crate::Encode;

pub struct Encoder<'buf> {
    buffer: &'buf mut [u8],
    position: usize,
}

impl<'buf> Encoder<'buf> {
    #[inline]
    pub fn new(buffer: &'buf mut [u8]) -> Self {
        Self {
            buffer,
            position: 0,
        }
    }

    #[inline]
    pub fn advance(&mut self, offset: usize) {
        self.position += offset
    }

    #[inline]
    pub fn position(&self) -> usize {
        self.position
    }

    #[inline]
    pub fn buffer_mut(&mut self) -> &mut [u8] {
        &mut self.buffer[self.position..]
    }
}

impl<'buf> Encode for Encoder<'buf> {
    #[inline]
    fn encode_sint32(&mut self, n: i32) {
        let (data, size) = encode_varint(((n << 1) ^ (n >> 31)) as u32 as u64);
        self.buffer_mut()[0..size].clone_from_slice(&data[0..size]);
        self.advance(size);
    }

    #[inline]
    fn encode_sint64(&mut self, n: i64) {
        let (data, size) = encode_varint(((n << 1) ^ (n >> 63)) as u64);
        self.buffer_mut()[0..size].clone_from_slice(&data[0..size]);
        self.advance(size);
    }

    #[inline]
    fn encode_uint32(&mut self, n: u32) {
        let (data, size) = encode_varint(u64::from(n));
        self.buffer_mut()[0..size].clone_from_slice(&data[0..size]);
        self.advance(size);
    }

    #[inline]
    fn encode_uint64(&mut self, n: u64) {
        let (data, size) = encode_varint(n);
        self.buffer_mut()[0..size].clone_from_slice(&data[0..size]);
        self.advance(size);
    }

    #[inline]
    fn encode_sfixed32(&mut self, n: i32) {
        self.buffer_mut()[0..std::mem::size_of::<i32>()].clone_from_slice(&n.to_le_bytes());
        self.advance(std::mem::size_of::<i32>());
    }

    #[inline]
    fn encode_sfixed64(&mut self, n: i64) {
        self.buffer_mut()[0..std::mem::size_of::<i64>()].clone_from_slice(&n.to_le_bytes());
        self.advance(std::mem::size_of::<i64>());
    }

    #[inline]
    fn encode_fixed32(&mut self, n: u32) {
        self.buffer_mut()[0..std::mem::size_of::<u32>()].clone_from_slice(&n.to_le_bytes());
        self.advance(std::mem::size_of::<u32>());
    }

    #[inline]
    fn encode_fixed64(&mut self, n: u64) {
        self.buffer_mut()[0..std::mem::size_of::<u64>()].clone_from_slice(&n.to_le_bytes());
        self.advance(std::mem::size_of::<u64>());
    }

    #[inline]
    fn encode_float(&mut self, n: f32) {
        self.buffer_mut()[0..std::mem::size_of::<f32>()].clone_from_slice(&n.to_le_bytes());
        self.advance(std::mem::size_of::<f32>());
    }

    #[inline]
    fn encode_double(&mut self, n: f64) {
        self.buffer_mut()[0..std::mem::size_of::<f64>()].clone_from_slice(&n.to_le_bytes());
        self.advance(std::mem::size_of::<f64>());
    }

    #[inline]
    fn encode_bytes(&mut self, b: &[u8]) {
        let len = b.len();
        #[allow(clippy::cast_possible_truncation)]
        self.encode_uint64(len as u64);
        self.buffer_mut()[0..len].clone_from_slice(b);
        self.advance(len);
    }
}

#[derive(Debug, Default)]
pub(crate) struct SizeHint {
    size: usize,
}

impl SizeHint {
    #[inline]
    pub fn size(&self) -> usize {
        self.size
    }
}

impl Encode for SizeHint {
    #[inline]
    fn encode_sint32(&mut self, n: i32) {
        let (_, size) = encode_varint(((n << 1) ^ (n >> 31)) as u32 as u64);
        self.size += size;
    }

    #[inline]
    fn encode_sint64(&mut self, n: i64) {
        let (_, size) = encode_varint(((n << 1) ^ (n >> 63)) as u64);
        self.size += size;
    }

    #[inline]
    fn encode_uint32(&mut self, n: u32) {
        let (_, size) = encode_varint(u64::from(n));
        self.size += size;
    }

    #[inline]
    fn encode_uint64(&mut self, n: u64) {
        let (_, size) = encode_varint(n);
        self.size += size;
    }

    #[inline]
    fn encode_sfixed32(&mut self, _n: i32) {
        self.size += std::mem::size_of::<i32>();
    }

    #[inline]
    fn encode_sfixed64(&mut self, _n: i64) {
        self.size += std::mem::size_of::<i64>();
    }

    #[inline]
    fn encode_fixed32(&mut self, _n: u32) {
        self.size += std::mem::size_of::<u32>();
    }

    #[inline]
    fn encode_fixed64(&mut self, _n: u64) {
        self.size += std::mem::size_of::<u64>();
    }

    #[inline]
    fn encode_float(&mut self, _n: f32) {
        self.size += std::mem::size_of::<f32>();
    }

    #[inline]
    fn encode_double(&mut self, _n: f64) {
        self.size += std::mem::size_of::<f64>();
    }

    #[inline]
    fn encode_bytes(&mut self, b: &[u8]) {
        #[allow(clippy::cast_possible_truncation)]
        self.encode_uint64(b.len() as u64);
        self.size += b.len();
    }
}

#[allow(clippy::cast_possible_truncation)] // Each step intentionally takes the low byte.
fn encode_varint(mut value: u64) -> ([u8; 10], usize) {
    let mut buffer = [0; 10];
    let mut size = 0;
    while value >= 0x80 {
        buffer[size] = (value as u8) | 0x80;
        value >>= 7;
        size += 1;
    }
    buffer[size] = value as u8;
    (buffer, size + 1)
}

// A growing encoder lets transports write safely without trusting a size hint.
impl Encode for Vec<u8> {
    fn encode_sint32(&mut self, n: i32) {
        self.encode_uint32(((n << 1) ^ (n >> 31)) as u32);
    }
    fn encode_sint64(&mut self, n: i64) {
        self.encode_uint64(((n << 1) ^ (n >> 63)) as u64);
    }
    fn encode_uint32(&mut self, n: u32) {
        self.encode_uint64(u64::from(n));
    }
    fn encode_uint64(&mut self, n: u64) {
        let (bytes, size) = encode_varint(n);
        self.extend_from_slice(&bytes[..size]);
    }
    fn encode_sfixed32(&mut self, n: i32) {
        self.extend_from_slice(&n.to_le_bytes());
    }
    fn encode_sfixed64(&mut self, n: i64) {
        self.extend_from_slice(&n.to_le_bytes());
    }
    fn encode_fixed32(&mut self, n: u32) {
        self.extend_from_slice(&n.to_le_bytes());
    }
    fn encode_fixed64(&mut self, n: u64) {
        self.extend_from_slice(&n.to_le_bytes());
    }
    fn encode_float(&mut self, n: f32) {
        self.extend_from_slice(&n.to_le_bytes());
    }
    fn encode_double(&mut self, n: f64) {
        self.extend_from_slice(&n.to_le_bytes());
    }
    fn encode_bytes(&mut self, bytes: &[u8]) {
        self.encode_uint64(bytes.len() as u64);
        self.extend_from_slice(bytes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Decode, decoder::Decoder};

    #[test]
    fn growing_encoder_handles_boundaries_and_large_bytes() {
        let mut encoded = Vec::new();
        for value in [0, 127, 128, u32::MAX as u64, u64::MAX] {
            encoded.encode_uint64(value);
        }
        encoded.encode_sint64(i64::MIN);
        let payload = vec![42; 64 * 1024];
        encoded.encode_bytes(&payload);
        let mut decoder = Decoder::new(&encoded);
        for value in [0, 127, 128, u32::MAX as u64, u64::MAX] {
            assert_eq!(decoder.decode_uint64().unwrap(), value);
        }
        assert_eq!(decoder.decode_sint64().unwrap(), i64::MIN);
        assert_eq!(decoder.decode_bytes().unwrap(), payload);
        assert!(decoder.eof());
    }
}
