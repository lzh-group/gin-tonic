use crate::{
    Decode, Message, ProtoError, Scalar, Tag,
    scalars::{Int32, Int64},
};

#[derive(Clone, Debug, Default)]
pub struct Duration {
    pub seconds: i64,
    pub nanos: i32,
}

#[derive(Default)]
struct DurationBuilder {
    seconds: Option<i64>,
    nanos: Option<i32>,
}

impl DurationBuilder {
    #[inline]
    fn finish(self) -> Result<Duration, ProtoError> {
        let Self { seconds, nanos } = self;
        Ok(Duration {
            seconds: seconds.unwrap_or_default(),
            nanos: nanos.unwrap_or_default(),
        })
    }

    #[inline]
    fn decode_field(&mut self, tag: Tag, decoder: &mut impl Decode) -> Result<(), ProtoError> {
        match tag.field_number() {
            1 => {
                tag.expect_wire_type(0)?;
                self.seconds = Some(Scalar::<Int64>::decode(decoder)?);
            }
            2 => {
                tag.expect_wire_type(0)?;
                self.nanos = Some(Scalar::<Int32>::decode(decoder)?);
            }
            _ => decoder.skip_field(tag)?,
        }
        Ok(())
    }
}

impl Message for Duration {
    fn encode_message(&self, encoder: &mut impl crate::Encode) {
        <i64 as Scalar<Int64>>::encode_field(&self.seconds, 1, encoder);
        <i32 as Scalar<Int32>>::encode_field(&self.nanos, 2, encoder);
    }

    fn decode_message(decoder: &mut impl crate::Decode) -> Result<Self, crate::ProtoError>
    where
        Self: Sized,
    {
        let mut builder = DurationBuilder::default();

        while !decoder.eof() {
            let tag = decoder.decode_tag()?;
            builder.decode_field(tag, decoder)?;
        }

        builder.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decoder::Decoder;

    #[test]
    fn duration_uses_scalar_defaults_and_skips_unknown_fields() {
        let empty = Duration::decode_message(&mut Decoder::new(&[])).unwrap();
        assert_eq!((empty.seconds, empty.nanos), (0, 0));
        let value = Duration::decode_message(&mut Decoder::new(&[24, 42, 8, 1])).unwrap();
        assert_eq!((value.seconds, value.nanos), (1, 0));
    }
}
