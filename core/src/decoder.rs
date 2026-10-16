use crate::{Decode, ProtoError};

pub struct Decoder<'buf> {
    buffer: &'buf [u8],
    position: usize,
}

impl<'buf> Decoder<'buf> {
    pub fn new(buffer: &'buf [u8]) -> Self {
        Self {
            buffer,
            position: 0,
        }
    }

    pub fn advance(&mut self, offset: usize) -> Result<(), ProtoError> {
        self.take(offset).map(|_| ())
    }

    pub fn position(&self) -> usize {
        self.position
    }
    pub fn buffer(&self) -> &'buf [u8] {
        &self.buffer[self.position..]
    }

    fn take(&mut self, size: usize) -> Result<&'buf [u8], ProtoError> {
        let end = self
            .position
            .checked_add(size)
            .ok_or(ProtoError::UnexpectedEof)?;
        let bytes = self
            .buffer
            .get(self.position..end)
            .ok_or(ProtoError::UnexpectedEof)?;
        self.position = end;
        Ok(bytes)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], ProtoError> {
        Ok(self.take(N)?.try_into()?)
    }
}

impl<'buf> Decode for Decoder<'buf> {
    fn eof(&self) -> bool {
        self.position == self.buffer.len()
    }
    fn position(&self) -> usize {
        self.position
    }
    fn len(&self) -> usize {
        self.buffer.len()
    }
    fn advance(&mut self, size: usize) -> Result<(), ProtoError> {
        self.advance(size)
    }
    fn sub_decoder(&mut self, size: usize) -> Result<impl Decode, ProtoError> {
        Ok(Decoder::new(self.take(size)?))
    }
    fn buffer(&self) -> &[u8] {
        self.buffer()
    }

    fn decode_sint32(&mut self) -> Result<i32, ProtoError> {
        let value = self.decode_uint32()?;
        Ok(((value >> 1) as i32) ^ -((value & 1) as i32))
    }
    fn decode_sint64(&mut self) -> Result<i64, ProtoError> {
        let value = self.decode_uint64()?;
        Ok(((value >> 1) as i64) ^ -((value & 1) as i64))
    }
    #[allow(clippy::cast_possible_truncation)] // Protobuf uint32 retains the low 32 bits.
    fn decode_uint32(&mut self) -> Result<u32, ProtoError> {
        Ok(self.decode_uint64()? as u32)
    }
    fn decode_uint64(&mut self) -> Result<u64, ProtoError> {
        let mut value = 0u64;
        for shift in (0..70).step_by(7) {
            let byte = self.take(1)?[0];
            if shift == 63 && byte > 1 {
                return Err(ProtoError::VarIntOverflow);
            }
            value |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Ok(value);
            }
        }
        Err(ProtoError::VarIntOverflow)
    }
    fn decode_sfixed32(&mut self) -> Result<i32, ProtoError> {
        Ok(i32::from_le_bytes(self.array()?))
    }
    fn decode_sfixed64(&mut self) -> Result<i64, ProtoError> {
        Ok(i64::from_le_bytes(self.array()?))
    }
    fn decode_fixed32(&mut self) -> Result<u32, ProtoError> {
        Ok(u32::from_le_bytes(self.array()?))
    }
    fn decode_fixed64(&mut self) -> Result<u64, ProtoError> {
        Ok(u64::from_le_bytes(self.array()?))
    }
    fn decode_float(&mut self) -> Result<f32, ProtoError> {
        Ok(f32::from_le_bytes(self.array()?))
    }
    fn decode_double(&mut self) -> Result<f64, ProtoError> {
        Ok(f64::from_le_bytes(self.array()?))
    }
    fn decode_bytes(&mut self) -> Result<Vec<u8>, ProtoError> {
        let size = usize::try_from(self.decode_uint64()?).map_err(|_| ProtoError::UnexpectedEof)?;
        Ok(self.take(size)?.to_vec())
    }
    fn decode_string(&mut self) -> Result<String, ProtoError> {
        let size = usize::try_from(self.decode_uint64()?).map_err(|_| ProtoError::UnexpectedEof)?;
        Ok(str::from_utf8(self.take(size)?)?.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncated_and_overflowing_inputs_are_errors() {
        assert!(Decoder::new(&[0x80]).decode_uint64().is_err());
        assert!(matches!(
            Decoder::new(&[0xff; 10]).decode_uint64(),
            Err(ProtoError::VarIntOverflow)
        ));
        assert!(Decoder::new(&[3, 1]).decode_bytes().is_err());
        assert!(Decoder::new(&[1]).decode_fixed32().is_err());
        assert!(Decoder::new(&[1, 0xff]).decode_string().is_err());
        assert!(Decoder::new(&[0]).decode_tag().is_err());
        assert!(Decoder::new(&[0x0e]).decode_tag().is_err());
        assert!(Decoder::new(&[]).advance(usize::MAX).is_err());
    }

    #[test]
    fn nested_decoder_stops_at_its_boundary() {
        let mut outer = Decoder::new(&[1, 0x80, 7]);
        let size = outer.decode_uint32().unwrap() as usize;
        let mut inner = outer.sub_decoder(size).unwrap();
        assert!(inner.decode_uint64().is_err());
        drop(inner);
        assert_eq!(outer.decode_uint32().unwrap(), 7);
    }

    #[test]
    fn unknown_fields_skip_their_complete_payload() {
        for (wire, bytes) in [
            (0, vec![0xac, 2]),
            (1, vec![0; 8]),
            (2, vec![3, 1, 2, 3]),
            (5, vec![0; 4]),
        ] {
            let mut bytes = bytes;
            bytes.extend([0x08, 0x01]);
            let mut decoder = Decoder::new(&bytes);
            decoder.skip_field(crate::Tag::from_parts(9, wire)).unwrap();
            assert_eq!(decoder.decode_tag().unwrap().field_number(), 1);
            assert!(decoder.decode_bool().unwrap());
        }
    }

    #[test]
    fn unknown_groups_are_matched_and_truncation_is_rejected() {
        let mut decoder = Decoder::new(&[19, 27, 32, 7, 28, 20, 8, 1]);
        let tag = decoder.decode_tag().unwrap();
        decoder.skip_field(tag).unwrap();
        assert_eq!(decoder.decode_tag().unwrap().field_number(), 1);
        assert!(decoder.decode_bool().unwrap());
        for wire in [&[19, 28][..], &[19][..], &[18, 5, 1][..]] {
            let mut decoder = Decoder::new(wire);
            let tag = decoder.decode_tag().unwrap();
            assert!(decoder.skip_field(tag).is_err());
        }
    }
}
