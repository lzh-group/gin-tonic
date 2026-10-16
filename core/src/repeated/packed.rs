use crate::{Decode, PackableMarker, Packed, Scalar, Tag, wire_types::WIRE_TYPE_LENGTH_ENCODED};

impl<RustType, ProtobufType> Packed<ProtobufType> for Vec<RustType>
where
    RustType: Scalar<ProtobufType> + PackableMarker<ProtobufType>,
{
    type Rust = RustType;

    #[inline]
    fn encode(&self, field_number: u32, encoder: &mut impl crate::Encode) {
        if self.is_empty() {
            return;
        }

        // packed fields are always length encoded
        encoder.encode_tag(Tag::from_parts(field_number, WIRE_TYPE_LENGTH_ENCODED));

        let mut size = 0;
        for i in self.iter() {
            size += Scalar::<ProtobufType>::size_hint(i);
        }

        encoder.encode_uint64(size as u64);
        for i in self.iter() {
            Scalar::<ProtobufType>::encode(i, encoder);
        }
    }

    #[inline]
    fn decode(tag: Tag, decoder: &mut impl Decode, v: &mut Self) -> Result<(), crate::ProtoError> {
        if tag.wire_type() == WIRE_TYPE_LENGTH_ENCODED {
            let size = usize::try_from(decoder.decode_uint64()?)
                .map_err(|_| crate::ProtoError::UnexpectedEof)?;
            let mut packed = decoder.sub_decoder(size)?;
            while !packed.eof() {
                v.push(Scalar::<ProtobufType>::decode(&mut packed)?);
            }
        } else {
            tag.expect_wire_type(<RustType as Scalar<ProtobufType>>::WIRE_TYPE)?;
            v.push(Scalar::<ProtobufType>::decode(decoder)?);
        }
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use std::fmt::Debug;

    use crate::{
        Decode, PackableMarker, Packed, Scalar, decoder::Decoder, encoder::Encoder, scalars::*,
    };

    #[test]
    fn packed() {
        fn inner<RustType, ProtobufType>(data: &Vec<RustType>, expected_bytes: &'static [u8])
        where
            RustType: Scalar<ProtobufType> + PackableMarker<ProtobufType> + PartialEq + Debug,
        {
            let size_hint = Packed::<ProtobufType>::size_hint(data, 1);

            let mut buffer = vec![0u8; size_hint];
            let mut encoder = Encoder::new(&mut buffer);
            Packed::<ProtobufType>::encode(data, 1, &mut encoder);

            assert_eq!(size_hint, buffer.len());
            assert_eq!(&expected_bytes[..size_hint], &buffer[..size_hint]);

            let mut decoder = Decoder::new(&buffer[..size_hint]);
            let tag = decoder.decode_tag().unwrap();
            let mut deserialized = vec![];
            <Vec<RustType> as Packed<ProtobufType>>::decode(tag, &mut decoder, &mut deserialized)
                .unwrap();

            assert_eq!(data, &deserialized)
        }

        inner::<i32, Int32>(
            &vec![1, 2, -3],
            b"\x0a\x0c\x01\x02\xfd\xff\xff\xff\xff\xff\xff\xff\xff\x01",
        );
        inner::<i64, Int64>(
            &vec![1, 2, -3],
            b"\x0a\x0c\x01\x02\xfd\xff\xff\xff\xff\xff\xff\xff\xff\x01",
        );

        inner::<u32, UInt32>(&vec![1, 2, 3], b"\x0a\x03\x01\x02\x03");
        inner::<u64, UInt64>(&vec![1, 2, 3], b"\x0a\x03\x01\x02\x03");

        inner::<i32, SInt32>(&vec![1, 2, -3], b"\x0a\x03\x02\x04\x05");
        inner::<i64, SInt64>(&vec![1, 2, -3], b"\x0a\x03\x02\x04\x05");

        inner::<i32, SFixed32>(
            &vec![1, 2, -3],
            b"\x0a\x0c\x01\x00\x00\x00\x02\x00\x00\x00\xfd\xff\xff\xff",
        );
        inner::<i64, SFixed64 >(
            &vec![1, 2, -3],
            b"\x0a\x18\x01\x00\x00\x00\x00\x00\x00\x00\x02\x00\x00\x00\x00\x00\x00\x00\xfd\xff\xff\xff\xff\xff\xff\xff",
        );

        inner::<u32, Fixed32>(
            &vec![1, 2, 3],
            b"\x0a\x0c\x01\x00\x00\x00\x02\x00\x00\x00\x03\x00\x00\x00",
        );
        inner::<u64,Fixed64, >(
            &vec![1, 2, 3],
            b"\x0a\x18\x01\x00\x00\x00\x00\x00\x00\x00\x02\x00\x00\x00\x00\x00\x00\x00\x03\x00\x00\x00\x00\x00\x00\x00",
        );

        inner::<f32, Float>(
            &vec![1.0, 2.0, -3.0],
            b"\x0a\x0c\x00\x00\x80\x3f\x00\x00\x00\x40\x00\x00\x40\xc0",
        );
        inner::<f64, Double>(
            &vec![1.0, 2.0, -3.0],
            b"\x0a\x18\x00\x00\x00\x00\x00\x00\xf0\x3f\x00\x00\x00\x00\x00\x00\x00\x40\x00\x00\x00\x00\x00\x00\x08\xc0",
        );

        inner::<bool, Bool>(&vec![true, false], b"\x0a\x02\x01\x00");
    }
}

#[cfg(test)]
mod compatibility_tests {
    use super::*;
    use crate::{
        ProtoError,
        decoder::Decoder,
        scalars::{Fixed32, UInt32},
    };

    #[test]
    fn mixed_packed_and_unpacked_segments_append() {
        let mut decoder = Decoder::new(&[8, 1, 10, 2, 2, 3, 8, 4]);
        let mut values = Vec::<u32>::new();
        while !decoder.eof() {
            let tag = decoder.decode_tag().unwrap();
            <Vec<u32> as Packed<UInt32>>::decode(tag, &mut decoder, &mut values).unwrap();
        }
        assert_eq!(values, [1, 2, 3, 4]);
    }

    #[test]
    fn truncated_packed_value_cannot_consume_following_field() {
        let mut decoder = Decoder::new(&[1, 0x80, 8, 1]);
        let mut values = Vec::<u32>::new();
        assert!(
            <Vec<u32> as Packed<UInt32>>::decode(Tag::from_parts(1, 2), &mut decoder, &mut values)
                .is_err()
        );
        assert_eq!(decoder.decode_tag().unwrap(), Tag::from_parts(1, 0));
        assert_eq!(decoder.decode_uint32().unwrap(), 1);
        let mut decoder = Decoder::new(&[3, 1, 2, 3]);
        assert!(matches!(
            <Vec<u32> as Packed<Fixed32>>::decode(
                Tag::from_parts(1, 2),
                &mut decoder,
                &mut Vec::new()
            ),
            Err(ProtoError::UnexpectedEof)
        ));
    }
}
