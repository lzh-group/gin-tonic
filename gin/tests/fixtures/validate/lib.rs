#![cfg(test)]
use gin_tonic::{Message, ProtoError, Scalar, Validate, decoder::Decoder, scalars::ProtoString};
include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
use sample::validate::v1::*;
use shared_fixture::shared::v1::Remote;

/// A custom `rust_type` message that does not implement `Validate`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Custom(String);

impl Message for Custom {
    fn encode_message(&self, encoder: &mut impl gin_tonic::Encode) {
        Scalar::<ProtoString>::encode_field(&self.0, 1, encoder);
    }
    fn decode_message(decoder: &mut impl gin_tonic::Decode) -> Result<Self, ProtoError> {
        let mut value = Self::default();
        while !decoder.eof() {
            let tag = decoder.decode_tag()?;
            value.0 = Scalar::<ProtoString>::decode(decoder)?;
            let _ = tag;
        }
        Ok(value)
    }
}

fn violation(value: &impl Validate) -> String {
    value.validate().unwrap_err().to_string()
}

fn numbers() -> Numbers {
    Numbers {
        int32: 0,
        int64: 1,
        uint32: 0,
        uint64: 11,
        sint32: -1,
        sint64: 1,
        fixed32: 1,
        fixed64: 8,
        sfixed32: -9,
        sfixed64: 0,
        ratio: 1.0,
        weight: -1.5,
        scale: 0.0,
        limit: None,
        side: Side::Short,
    }
}

#[test]
fn numeric_bounds() {
    assert_eq!(numbers().validate(), Ok(()));
    assert_eq!(Numbers { uint64: 4, ..numbers() }.validate(), Ok(()));
    let cases: [(Numbers, &str); 17] = [
        (Numbers { int32: -6, ..numbers() }, "field `int32` must be >= -5"),
        (Numbers { int32: 11, ..numbers() }, "field `int32` must be <= 10"),
        (Numbers { int64: 0, ..numbers() }, "field `int64` must be > 0"),
        (Numbers { uint32: 100, ..numbers() }, "field `uint32` must be < 100"),
        (Numbers { uint64: 7, ..numbers() }, "field `uint64` must be > 10 or < 5"),
        (Numbers { sint32: -2, ..numbers() }, "field `sint32` must be >= -1"),
        (Numbers { sint64: 2, ..numbers() }, "field `sint64` must be <= 1"),
        (Numbers { fixed32: 0, ..numbers() }, "field `fixed32` must be >= 1"),
        (Numbers { fixed64: 9, ..numbers() }, "field `fixed64` must be < 9"),
        (Numbers { sfixed32: -10, ..numbers() }, "field `sfixed32` must be > -10"),
        (Numbers { sfixed64: 1, ..numbers() }, "field `sfixed64` must be < 1"),
        (Numbers { ratio: 0.0, ..numbers() }, "field `ratio` must be > 0"),
        (Numbers { ratio: 1.5, ..numbers() }, "field `ratio` must be <= 1"),
        (Numbers { ratio: f64::NAN, ..numbers() }, "field `ratio` must be > 0"),
        (Numbers { weight: -2.0, ..numbers() }, "field `weight` must be >= -1.5"),
        (Numbers { scale: f32::INFINITY, ..numbers() }, "field `scale` must be finite"),
        (Numbers { limit: Some(-0.5), ..numbers() }, "field `limit` must be >= 0"),
    ];
    for (value, expected) in cases {
        assert_eq!(violation(&value), expected);
    }
    assert_eq!(Numbers { weight: f32::MAX, ..numbers() }.validate(), Ok(()));
    assert_eq!(Numbers { limit: Some(0.0), ..numbers() }.validate(), Ok(()));
}

#[test]
fn string_rules() {
    let text = Text {
        name: "äöüß".into(),
        id: "123e4567-e89b-12d3-a456-426614174000".into(),
        note: None,
    };
    assert_eq!(text.validate(), Ok(()));
    let cases = [
        (Text { name: String::new(), ..text.clone() }, "field `name` must be at least 1 characters"),
        (Text { name: "abcde".into(), ..text.clone() }, "field `name` must be at most 4 characters"),
        (Text { id: String::new(), ..text.clone() }, "field `id` must be a valid UUID"),
        (Text { id: "not-a-uuid".into(), ..text.clone() }, "field `id` must be a valid UUID"),
        (Text { note: Some("long".into()), ..text.clone() }, "field `note` must be at most 3 characters"),
    ];
    for (value, expected) in cases {
        assert_eq!(violation(&value), expected);
    }
}

fn collections() -> Collections {
    Collections {
        ids: vec!["ab".into(), "cd".into()],
        values: vec![1.0, 2.0],
        lots: [("cu".to_owned(), 1)].into_iter().collect(),
        tas: vec![Ta { period: 1 }],
        ta_map: [(5, Ta { period: 2 })].into_iter().collect(),
        ranks: vec![1, 2],
    }
}

#[test]
fn repeated_and_map_rules() {
    assert_eq!(collections().validate(), Ok(()));
    let mut too_many_lots = collections();
    too_many_lots.lots.extend([("al".to_owned(), 1), ("zn".to_owned(), 1)]);
    let cases = [
        (Collections { ids: vec![], ..collections() }, "field `ids` must contain at least 1 item(s)"),
        (
            Collections { ids: vec!["ab".into(), "cd".into(), "ef".into(), "gh".into()], ..collections() },
            "field `ids` must contain at most 3 item(s)",
        ),
        (Collections { ids: vec!["ab".into(), "ab".into()], ..collections() }, "field `ids` must contain unique items"),
        (Collections { ids: vec!["ab".into(), "c".into()], ..collections() }, "field `ids[1]` must be at least 2 characters"),
        (Collections { values: vec![1.0, -0.0, 0.0], ..collections() }, "field `values` must contain unique items"),
        (Collections { values: vec![1.0, f64::NAN], ..collections() }, "field `values[1]` must be finite"),
        (Collections { lots: Default::default(), ..collections() }, "field `lots` must contain at least 1 pair(s)"),
        (too_many_lots, "field `lots` must contain at most 2 pair(s)"),
        (
            Collections { lots: [("c".to_owned(), 1)].into_iter().collect(), ..collections() },
            "map key `lots[\"c\"]` must be at least 2 characters",
        ),
        (
            Collections { lots: [("cu".to_owned(), 0)].into_iter().collect(), ..collections() },
            "field `lots[\"cu\"]` must be > 0",
        ),
        (Collections { tas: vec![Ta { period: 1 }; 3], ..collections() }, "field `tas` must contain at most 2 item(s)"),
        (
            Collections { tas: vec![Ta { period: 1 }, Ta { period: 0 }], ..collections() },
            "field `tas[1].period` must be >= 1",
        ),
        (
            Collections { ta_map: [(5, Ta { period: 10001 })].into_iter().collect(), ..collections() },
            "field `ta_map[5].period` must be <= 10000",
        ),
        (Collections { ranks: vec![3, 3], ..collections() }, "field `ranks` must contain unique items"),
    ];
    for (value, expected) in cases {
        assert_eq!(violation(&value), expected);
    }
    let error = Collections { ids: vec!["ab".into(), "c".into()], ..collections() }
        .validate()
        .unwrap_err();
    assert_eq!(error.field_path(), "ids[1]");
    assert_eq!(*error.violation(), gin_tonic::validate::Violation::MinLen(2));
}

#[test]
fn nested_messages_recurse() {
    let rule = Rule { ta: Ta { period: 14 }, fallback: Some(Ta { period: 7 }) };
    assert_eq!(rule.validate(), Ok(()));
    assert_eq!(
        violation(&Rule { ta: Ta { period: 10001 }, ..rule }),
        "field `ta.period` must be <= 10000"
    );
    assert_eq!(
        violation(&Rule { fallback: Some(Ta { period: 0 }), ..rule }),
        "field `fallback.period` must be >= 1"
    );
    let node = Node {
        depth: 0,
        child: Some(Box::new(Node { depth: 1, child: Some(Box::new(Node { depth: 4, child: None })) })),
    };
    assert_eq!(violation(&node), "field `child.child.depth` must be <= 3");
}

#[test]
fn oneofs_recurse_with_variant_names() {
    let valid = Mixed { sequence: 1, content: mixed::Content::Text("x".into()) };
    assert_eq!(valid.validate(), Ok(()));
    assert_eq!(
        violation(&Mixed { sequence: 1, content: mixed::Content::Text(String::new()) }),
        "field `text` must be at least 1 characters"
    );
    let rule = Rule { ta: Ta { period: 0 }, fallback: None };
    assert_eq!(
        violation(&Mixed { sequence: 1, content: mixed::Content::Rule(rule) }),
        "field `rule.ta.period` must be >= 1"
    );
    assert_eq!(Choice::Score(1.0).validate(), Ok(()));
    assert_eq!(violation(&Choice::Score(f64::NAN)), "field `score` must be finite");
    assert_eq!(violation(&Choice::Ta(Ta { period: 0 })), "field `ta.period` must be >= 1");
    assert_eq!(
        violation(&Choice::Remote(Remote { code: String::new() })),
        "field `remote.code` must be at least 1 characters"
    );
}

#[test]
fn extern_path_messages_recurse() {
    let remote = || Remote { code: "x".into() };
    let foreign = Foreign {
        remote: remote(),
        remotes: vec![remote()],
        by_code: [("x".to_owned(), remote())].into_iter().collect(),
        custom: Custom(String::new()),
        choice: Choice::Score(0.0),
    };
    assert_eq!(foreign.validate(), Ok(()));
    let empty = || Remote { code: String::new() };
    let cases = [
        (Foreign { remote: empty(), ..foreign.clone() }, "field `remote.code` must be at least 1 characters"),
        (Foreign { remotes: vec![remote(), empty()], ..foreign.clone() }, "field `remotes[1].code` must be at least 1 characters"),
        (
            Foreign { by_code: [("y".to_owned(), empty())].into_iter().collect(), ..foreign.clone() },
            "field `by_code[\"y\"].code` must be at least 1 characters",
        ),
        (Foreign { choice: Choice::Remote(empty()), ..foreign.clone() }, "field `choice.remote.code` must be at least 1 characters"),
    ];
    for (value, expected) in cases {
        assert_eq!(violation(&value), expected);
    }
}

#[test]
fn decoding_keeps_values_for_validation() {
    let mut wire = Vec::new();
    Ta { period: 0 }.encode_message(&mut wire);
    let decoded = Ta::decode_message(&mut Decoder::new(&wire)).unwrap();
    assert_eq!(violation(&decoded), "field `period` must be >= 1");
}
