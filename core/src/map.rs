use std::hash::Hash;

use fxhash::FxHashMap;

use crate::{
    Decode, Encode, Map, Scalar, Tag, encoder::SizeHint, error::ProtoError,
    wire_types::WIRE_TYPE_LENGTH_ENCODED,
};

pub struct KeyValuePairView<'p, RustKey, RustValue> {
    pub key: &'p RustKey,
    pub value: &'p RustValue,
}

impl<'p, RustKey, RustValue> KeyValuePairView<'p, RustKey, RustValue> {
    fn size_hint<ProtobufKey, ProtobufValue>(&self) -> usize
    where
        RustKey: Scalar<ProtobufKey>,
        RustValue: Scalar<ProtobufValue>,
    {
        let mut hint = SizeHint::default();
        self.encode(&mut hint);
        hint.size()
    }

    fn encode<ProtobufKey, ProtobufValue>(&self, encoder: &mut impl Encode)
    where
        RustKey: Scalar<ProtobufKey>,
        RustValue: Scalar<ProtobufValue>,
    {
        let tag = Tag::from_parts(1u32, <RustKey as Scalar<ProtobufKey>>::WIRE_TYPE);
        encoder.encode_tag(tag);
        <RustKey as Scalar<ProtobufKey>>::encode(self.key, encoder);

        let tag = Tag::from_parts(2u32, <RustValue as Scalar<ProtobufValue>>::WIRE_TYPE);
        encoder.encode_tag(tag);
        <RustValue as Scalar<ProtobufValue>>::encode(self.value, encoder);
    }
}

fn decode_entry<K, PK, V, PV>(decoder: &mut impl Decode) -> Result<(K, V), ProtoError>
where
    K: Scalar<PK> + Default,
    V: Scalar<PV> + Default,
{
    let size = usize::try_from(decoder.decode_uint64()?).map_err(|_| ProtoError::UnexpectedEof)?;
    let mut entry = decoder.sub_decoder(size)?;
    let mut key = K::default();
    let mut value = V::default();
    while !entry.eof() {
        let tag = entry.decode_tag()?;
        match tag.field_number() {
            1 => {
                tag.expect_wire_type(K::WIRE_TYPE)?;
                key = K::decode(&mut entry)?;
            }
            2 => {
                tag.expect_wire_type(V::WIRE_TYPE)?;
                value = V::decode(&mut entry)?;
            }
            _ => entry.skip_field(tag)?,
        }
    }
    Ok((key, value))
}

impl<K, PK, V, PV> Map<PK, PV> for FxHashMap<K, V>
where
    K: Scalar<PK> + Default + Hash + Eq,
    V: Scalar<PV> + Default,
{
    fn encode(&self, field_number: u32, encoder: &mut impl Encode) {
        for (key, value) in self {
            encoder.encode_tag(Tag::from_parts(field_number, WIRE_TYPE_LENGTH_ENCODED));
            let pair = KeyValuePairView { key, value };
            encoder.encode_uint64(pair.size_hint() as u64);
            pair.encode(encoder);
        }
    }
    fn decode(decoder: &mut impl Decode, map: &mut Self) -> Result<(), ProtoError> {
        let (key, value) = decode_entry(decoder)?;
        map.insert(key, value);
        Ok(())
    }
}

impl<K, PK, V, PV> Map<PK, PV> for std::collections::BTreeMap<K, V>
where
    K: Scalar<PK> + Default + Ord,
    V: Scalar<PV> + Default,
{
    fn encode(&self, field_number: u32, encoder: &mut impl Encode) {
        for (key, value) in self {
            encoder.encode_tag(Tag::from_parts(field_number, WIRE_TYPE_LENGTH_ENCODED));
            let pair = KeyValuePairView { key, value };
            encoder.encode_uint64(pair.size_hint() as u64);
            pair.encode(encoder);
        }
    }
    fn decode(decoder: &mut impl Decode, map: &mut Self) -> Result<(), ProtoError> {
        let (key, value) = decode_entry(decoder)?;
        map.insert(key, value);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        decoder::Decoder,
        scalars::{Int32, ProtoString},
    };
    use std::collections::BTreeMap;

    #[test]
    fn map_entries_allow_defaults_reordering_and_unknown_fields() {
        let mut map = BTreeMap::<String, i32>::new();
        // Empty entry: omitted scalar key/value take their protobuf defaults.
        <BTreeMap<_, _> as Map<ProtoString, Int32>>::decode(&mut Decoder::new(&[0]), &mut map)
            .unwrap();
        assert_eq!(map.get(""), Some(&0));
        // Value first, unknown field, then key.
        let entry = [9, 16, 7, 24, 99, 10, 3, b'k', b'e', b'y'];
        <BTreeMap<_, _> as Map<ProtoString, Int32>>::decode(&mut Decoder::new(&entry), &mut map)
            .unwrap();
        assert_eq!(map.get("key"), Some(&7));
        let replacement = [7, 10, 3, b'k', b'e', b'y', 16, 8];
        <BTreeMap<_, _> as Map<ProtoString, Int32>>::decode(
            &mut Decoder::new(&replacement),
            &mut map,
        )
        .unwrap();
        assert_eq!(map.get("key"), Some(&8));
        let mut encoded = Vec::new();
        <BTreeMap<_, _> as Map<ProtoString, Int32>>::encode(&map, 1, &mut encoded);
        let mut decoder = Decoder::new(&encoded);
        let mut hash = FxHashMap::<String, i32>::default();
        while !decoder.eof() {
            decoder.decode_tag().unwrap().expect_wire_type(2).unwrap();
            <FxHashMap<_, _> as Map<ProtoString, Int32>>::decode(&mut decoder, &mut hash).unwrap();
        }
        assert_eq!(hash.get("key"), Some(&8));
        assert_eq!(hash.get(""), Some(&0));
    }

    #[test]
    fn malformed_map_entry_is_an_error() {
        let mut map = FxHashMap::<String, i32>::default();
        for wire in [&[5, 10, 4, b'a'][..], &[1, 8][..], &[1, 0][..]] {
            assert!(
                <FxHashMap<_, _> as Map<ProtoString, Int32>>::decode(
                    &mut Decoder::new(wire),
                    &mut map
                )
                .is_err()
            );
        }
        assert!(map.is_empty());
    }
}
