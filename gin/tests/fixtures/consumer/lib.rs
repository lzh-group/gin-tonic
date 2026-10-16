#![cfg(test)]
use gin_tonic::{Message, ProtoError, decoder::Decoder};
include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
use fixture::v1::*;

fn decode<T: Message>(wire: &[u8]) -> Result<T, ProtoError> {
    T::decode_message(&mut Decoder::new(wire))
}
fn encode(value: &impl Message) -> Vec<u8> {
    let mut wire = Vec::new();
    value.encode_message(&mut wire);
    wire
}

#[test]
fn generated_values_preserve_presence_and_defaults() {
    // The nested child is present but has no encoded nonzero scalars.
    let mut value = decode::<Values>(&[10, 0]).unwrap();
    let _: &Child = &value.required_child;
    let _: &Option<Child> = &value.optional_child;
    let _: &State = &value.state;
    assert_eq!(value.required_child.count, 0);
    assert_eq!(value.state, State::Unspecified);
    assert!(!value.enabled);
    assert!(value.switch.is_none());
    assert!(value.optional_child.is_none());
    assert!(matches!(
        decode::<Values>(&[]),
        Err(ProtoError::MissingField(1))
    ));
    assert!(matches!(
        decode::<Values>(&[10, 0, 24, 99]),
        Err(ProtoError::UnknownEnumVariant(99))
    ));
    let omitted_map_value = decode::<Values>(&[10, 0, 58, 3, 10, 1, b'x']).unwrap();
    assert_eq!(omitted_map_value.counts.get("x"), Some(&0));
    value.optional_child = Some(Child { count: 0 });
    value.state = State::Ready;
    value.enabled = false;
    value.switch = Some(false);
    value.data = vec![0, 255];
    value.chunks = vec![vec![], vec![1, 2]];
    value.counts = [("x".to_owned(), 0)]
        .into_iter()
        .collect::<std::collections::BTreeMap<_, _>>();
    value.states = vec![State::Unspecified, State::Ready];
    let copied = decode::<Values>(&encode(&value)).unwrap();
    assert!(copied.optional_child.is_some());
    assert_eq!(copied.switch, Some(false));
    assert_eq!(copied.data, value.data);
    assert_eq!(copied.chunks, value.chunks);
    assert_eq!(copied.counts, value.counts);
    assert_eq!(copied.states, value.states);
    let stored = serde_json::to_string(&copied).unwrap();
    assert!(stored.contains("\"weights\":{\"x\":0}"));
    let without_map = stored.replace("\"weights\":{\"x\":0},", "");
    assert!(
        serde_json::from_str::<Values>(&without_map)
            .unwrap()
            .counts
            .is_empty()
    );
}

#[test]
fn generated_mixed_and_standalone_oneofs_compile_and_decode() {
    let value = Mixed {
        sequence: 0,
        content: mixed::Content::Text(String::new()),
    };
    let decoded = decode::<Mixed>(&encode(&value)).unwrap();
    assert!(matches!(decoded.content, mixed::Content::Text(value) if value.is_empty()));
    assert!(matches!(
        decode::<Mixed>(&[]),
        Err(ProtoError::MissingField(0))
    ));
    let value = Choice::Score(1.5);
    assert!(matches!(
        decode::<Choice>(&encode(&value)).unwrap(),
        Choice::Score(1.5)
    ));
    let record = Choice::Record(Child { count: 7 });
    assert!(matches!(
        decode::<Choice>(&encode(&record)).unwrap(),
        Choice::Record(Child { count: 7 })
    ));
}
