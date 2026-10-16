use std::{array::TryFromSliceError, str::Utf8Error};

#[derive(Debug, thiserror::Error)]
pub enum ProtoError {
    #[error("Unexpected end of protobuf input")]
    UnexpectedEof,
    #[error("Protobuf varint exceeds 64 bits")]
    VarIntOverflow,
    #[error("Invalid protobuf tag {0}")]
    InvalidTag(u64),
    #[error("Expected wire type {expected}, got {actual}")]
    UnexpectedWireType { expected: u8, actual: u8 },
    #[error(transparent)]
    Utf8(#[from] Utf8Error),
    #[error(transparent)]
    ArrayFromSlice(#[from] TryFromSliceError),
    #[error("Field number {0} is missing")]
    MissingField(u32),
    #[error("No OneOf variants found, expected one of: {0:?}")]
    MissingOneOf(&'static [u32]),
    #[error("{0} is not a known enum variant")]
    UnknownEnumVariant(i32),
    #[error("{0}")]
    Custom(String),
}
