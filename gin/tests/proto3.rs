use gin_tonic::{Enumeration, Message, OneOf, ProtoError, decoder::Decoder, encoder::Encoder};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Default, PartialEq, Enumeration)]
enum State {
    #[default]
    #[gin(id = 0)]
    Unspecified,
    #[gin(id = 7)]
    Ready,
}

#[derive(Clone, Debug, Default, PartialEq, Message)]
struct Values {
    #[gin(id = 1, scalar = "int32", default)]
    count: i32,
    #[gin(id = 2, scalar = "int32", default)]
    state: State,
    #[gin(id = 3, scalar = "bytes", default)]
    data: Vec<u8>,
    #[gin(id = 4, scalar = "int32")]
    states: Vec<State>,
    #[gin(id = 5, key_scalar = "string", value_scalar = "int32")]
    weights: BTreeMap<String, i32>,
    #[gin(id = 6, scalar = "bytes")]
    chunks: Vec<Vec<u8>>,
    #[gin(id = 7, scalar = "double")]
    optional: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, OneOf)]
enum Outcome {
    #[gin(id = 2, scalar = "double")]
    Score(f64),
    #[gin(id = 3, scalar = "bytes")]
    Record(Box<Values>),
}

#[derive(Clone, Debug, PartialEq, Message)]
struct Mixed {
    #[gin(id = 1, scalar = "uint64", default)]
    id: u64,
    #[gin(id = 0, oneof)]
    outcome: Outcome,
}

#[derive(Clone, Debug, PartialEq, Message)]
struct Nested {
    #[gin(id = 1)]
    value: Values,
}

fn decode<T: Message>(bytes: &[u8]) -> Result<T, ProtoError> {
    T::decode_message(&mut Decoder::new(bytes))
}
fn encode<T: Message>(value: &T) -> Vec<u8> {
    let mut bytes = vec![0; value.message_size_hint()];
    value.encode_message(&mut Encoder::new(&mut bytes));
    bytes
}

#[test]
fn implicit_defaults_are_distinct_from_required_nested_messages() {
    assert_eq!(decode::<Values>(&[]).unwrap(), Values::default());
    assert!(matches!(
        decode::<Nested>(&[]),
        Err(ProtoError::MissingField(1))
    ));
    assert_eq!(decode::<Nested>(&[10, 0]).unwrap().value, Values::default());
}

#[test]
fn enums_keep_zero_and_reject_unknown_numbers() {
    assert_eq!(State::try_from(0).unwrap(), State::Unspecified);
    assert_eq!(i32::from(State::Ready), 7);
    assert!(matches!(
        State::try_from(9),
        Err(ProtoError::UnknownEnumVariant(9))
    ));
    assert!(matches!(
        decode::<Values>(&[16, 9]),
        Err(ProtoError::UnknownEnumVariant(9))
    ));
}

#[test]
fn bytes_are_scalar_and_repeated_bytes_are_separate_values() {
    let value = Values {
        data: vec![0, 255],
        chunks: vec![vec![], vec![1, 2]],
        ..Default::default()
    };
    assert_eq!(decode::<Values>(&encode(&value)).unwrap(), value);
    assert_eq!(decode::<Values>(&[26, 2, 0, 255]).unwrap().data, [0, 255]);
}

#[test]
fn packed_and_unpacked_enums_can_be_interleaved() {
    let value = decode::<Values>(&[32, 7, 34, 2, 0, 7, 32, 0]).unwrap();
    assert_eq!(
        value.states,
        [
            State::Ready,
            State::Unspecified,
            State::Ready,
            State::Unspecified
        ]
    );
    assert_eq!(decode::<Values>(&encode(&value)).unwrap(), value);
}

#[test]
fn maps_restore_omitted_zero_values_and_unknown_fields_are_skipped() {
    let value = decode::<Values>(&[42, 3, 10, 1, b'x', 80, 1]).unwrap();
    assert_eq!(value.weights.get("x"), Some(&0));
    assert_eq!(decode::<Values>(&encode(&value)).unwrap(), value);
}

#[test]
fn mixed_and_standalone_oneofs_preserve_the_last_known_variant() {
    let value = Mixed {
        id: 0,
        outcome: Outcome::Record(Box::default()),
    };
    assert_eq!(decode::<Mixed>(&encode(&value)).unwrap(), value);
    assert!(matches!(
        decode::<Mixed>(&[]),
        Err(ProtoError::MissingField(0))
    ));
    let score = Outcome::Score(1.25);
    let mut bytes = encode(&score);
    bytes.extend_from_slice(&[80, 1]);
    assert_eq!(decode::<Outcome>(&bytes).unwrap(), score);
    assert!(decode::<Values>(&[13, 0, 0, 0, 0]).is_err());
}

#[derive(Clone, Debug, PartialEq, OneOf)]
enum Pick {
    #[gin(id = 1, scalar = "double", validate(finite))]
    LongScore(f64),
    #[gin(id = 2, scalar = "bytes", name = "item", validate(nested))]
    Record(Box<Limited>),
}

#[derive(Clone, Debug, Default, PartialEq, Message)]
struct Limited {
    #[gin(id = 1, scalar = "sint32", default, validate(gt = -1, lt = 3))]
    level: i32,
    #[gin(id = 2, validate(min_items = 1, items(max_len = 2)))]
    tags: Vec<String>,
    #[gin(
        id = 3,
        key_scalar = "int32",
        value_scalar = "double",
        validate(values(lte = 1.0))
    )]
    shares: BTreeMap<i32, f64>,
}

#[test]
fn hand_written_messages_validate_through_attributes() {
    use gin_tonic::Validate;

    assert_eq!(Values::default().validate(), Ok(()));
    assert_eq!(
        Mixed {
            id: 0,
            outcome: Outcome::Record(Box::default())
        }
        .validate(),
        Ok(())
    );
    let limited = Limited {
        level: 2,
        tags: vec!["ab".into()],
        shares: [(1, 1.0)].into_iter().collect(),
    };
    assert_eq!(limited.validate(), Ok(()));
    assert_eq!(
        Pick::LongScore(f64::NAN)
            .validate()
            .unwrap_err()
            .to_string(),
        "field `long_score` must be finite"
    );
    let invalid = [
        (
            Limited {
                level: 3,
                ..limited.clone()
            },
            "field `item.level` must be < 3",
        ),
        (
            Limited {
                tags: vec![],
                ..limited.clone()
            },
            "field `item.tags` must contain at least 1 item(s)",
        ),
        (
            Limited {
                tags: vec!["abc".into()],
                ..limited.clone()
            },
            "field `item.tags[0]` must be at most 2 characters",
        ),
        (
            Limited {
                shares: [(-4, 1.5)].into_iter().collect(),
                ..limited.clone()
            },
            "field `item.shares[-4]` must be <= 1",
        ),
    ];
    for (value, expected) in invalid {
        let value = Pick::Record(Box::new(value));
        assert_eq!(value.validate().unwrap_err().to_string(), expected);
        assert_eq!(decode::<Pick>(&encode(&value)).unwrap(), value);
    }
}
