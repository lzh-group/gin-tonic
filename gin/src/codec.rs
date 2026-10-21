//! [tonic::codec::Codec] implementation for gin-tonic

use std::marker::PhantomData;

use bytes::{Buf, BufMut};
use gin_tonic_core::{Message, Validate, decoder::Decoder};
use tonic::codec::{DecodeBuf, EncodeBuf};

#[derive(Debug, Clone)]
pub struct GinCodec<T, U> {
    _pd: PhantomData<(T, U)>,
}

impl<T, U> Default for GinCodec<T, U> {
    fn default() -> Self {
        Self { _pd: PhantomData }
    }
}

#[derive(Debug, Clone)]
pub struct GinEncoder<T> {
    _pd: PhantomData<T>,
}

#[derive(Debug, Clone)]
pub struct GinDecoder<U> {
    _pd: PhantomData<U>,
}

impl<T, U> tonic::codec::Codec for GinCodec<T, U>
where
    T: Message + Send + 'static + std::fmt::Debug,
    U: Message + Validate + Send + 'static + std::fmt::Debug,
{
    type Encode = T;
    type Decode = U;
    type Encoder = GinEncoder<T>;
    type Decoder = GinDecoder<U>;

    fn encoder(&mut self) -> Self::Encoder {
        GinEncoder { _pd: PhantomData }
    }

    fn decoder(&mut self) -> Self::Decoder {
        GinDecoder { _pd: PhantomData }
    }
}

impl<T: Message + std::fmt::Debug> tonic::codec::Encoder for GinEncoder<T> {
    type Item = T;
    type Error = tonic::Status;

    fn encode(&mut self, item: Self::Item, dst: &mut EncodeBuf<'_>) -> Result<(), Self::Error> {
        let encoded = encode_payload(item);
        dst.put_slice(&encoded);

        Ok(())
    }
}

impl<U: Message + Validate + std::fmt::Debug> tonic::codec::Decoder for GinDecoder<U> {
    type Item = U;
    type Error = tonic::Status;

    fn decode(&mut self, src: &mut DecodeBuf<'_>) -> Result<Option<Self::Item>, Self::Error> {
        let bytes = src.copy_to_bytes(src.remaining());
        let decoded = decode_payload::<U>(&bytes)?;

        Ok(Some(decoded))
    }
}

fn map_core_err(err: gin_tonic_core::ProtoError) -> tonic::Status {
    tonic::Status::invalid_argument(err.to_string())
}

fn encode_payload<T: Message>(item: T) -> Vec<u8> {
    let mut encoded = Vec::new();
    item.encode_message(&mut encoded);
    encoded
}

/// Decoded messages are validated against their protovalidate field rules.
fn decode_payload<U: Message + Validate>(bytes: &[u8]) -> Result<U, tonic::Status> {
    let decoded = U::decode_message(&mut Decoder::new(bytes)).map_err(map_core_err)?;
    decoded
        .validate()
        .map_err(|error| tonic::Status::invalid_argument(error.to_string()))?;
    Ok(decoded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gin_tonic_core::{Decode, Encode};

    #[derive(Debug)]
    struct Payload(Vec<u8>);
    impl Message for Payload {
        fn message_size_hint(&self) -> usize {
            0
        }
        fn encode_message(&self, encoder: &mut impl Encode) {
            encoder.encode_bytes(&self.0);
        }
        fn decode_message(decoder: &mut impl Decode) -> Result<Self, gin_tonic_core::ProtoError> {
            decoder.decode_bytes().map(Self)
        }
    }
    impl Validate for Payload {
        fn validate(&self) -> Result<(), gin_tonic_core::ValidationError> {
            Ok(())
        }
    }

    #[derive(Debug, crate::Message)]
    #[gin(root = "crate")]
    struct Limits {
        #[gin(id = 1, scalar = "int32", default, validate(gte = 1, lte = 10000))]
        period: i32,
        #[gin(id = 2, validate(nested))]
        child: Child,
    }

    #[derive(Debug, crate::Message)]
    #[gin(root = "crate")]
    struct Child {
        #[gin(id = 1, default, validate(min_len = 1))]
        name: String,
    }

    #[test]
    fn encoding_grows_without_trusting_the_size_hint() {
        let payload = vec![42; 64 * 1024];
        let encoded = encode_payload(Payload(payload.clone()));
        assert_eq!(decode_payload::<Payload>(&encoded).unwrap().0, payload);
    }

    #[test]
    fn malformed_input_returns_invalid_argument() {
        assert_eq!(
            decode_payload::<Payload>(&[0x80]).unwrap_err().code(),
            tonic::Code::InvalidArgument
        );
        assert_eq!(
            decode_payload::<Payload>(&[10, 1]).unwrap_err().code(),
            tonic::Code::InvalidArgument
        );
    }

    #[test]
    fn decoded_messages_are_validated() {
        let valid = Limits {
            period: 10000,
            child: Child { name: "x".into() },
        };
        let decoded = decode_payload::<Limits>(&encode_payload(valid)).unwrap();
        assert_eq!(decoded.period, 10000);

        let invalid = Limits {
            period: 10001,
            child: Child { name: "x".into() },
        };
        let status = decode_payload::<Limits>(&encode_payload(invalid)).unwrap_err();
        assert_eq!(status.code(), tonic::Code::InvalidArgument);
        assert_eq!(status.message(), "field `period` must be <= 10000");

        let nested = Limits {
            period: 1,
            child: Child {
                name: String::new(),
            },
        };
        let status = decode_payload::<Limits>(&encode_payload(nested)).unwrap_err();
        assert_eq!(status.code(), tonic::Code::InvalidArgument);
        assert_eq!(
            status.message(),
            "field `child.name` must be at least 1 characters"
        );
    }
}
