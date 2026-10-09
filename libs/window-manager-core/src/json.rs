//! Strict object-key validation precedes typed decoding at external JSON boundaries.
use serde::de::{DeserializeOwned, Error, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};
use std::collections::BTreeSet;
use std::fmt;
use std::io::Read;
use std::path::Path;

struct UniqueKeys;
impl<'de> Deserialize<'de> for UniqueKeys {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct KeysVisitor;
        impl<'de> Visitor<'de> for KeysVisitor {
            type Value = UniqueKeys;
            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("JSON with unique object keys")
            }
            fn visit_bool<E: Error>(self, _: bool) -> Result<UniqueKeys, E> {
                Ok(UniqueKeys)
            }
            fn visit_i64<E: Error>(self, _: i64) -> Result<UniqueKeys, E> {
                Ok(UniqueKeys)
            }
            fn visit_u64<E: Error>(self, _: u64) -> Result<UniqueKeys, E> {
                Ok(UniqueKeys)
            }
            fn visit_f64<E: Error>(self, _: f64) -> Result<UniqueKeys, E> {
                Ok(UniqueKeys)
            }
            fn visit_str<E: Error>(self, _: &str) -> Result<UniqueKeys, E> {
                Ok(UniqueKeys)
            }
            fn visit_unit<E: Error>(self) -> Result<UniqueKeys, E> {
                Ok(UniqueKeys)
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<UniqueKeys, A::Error> {
                while sequence.next_element::<UniqueKeys>()?.is_some() {}
                Ok(UniqueKeys)
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<UniqueKeys, A::Error> {
                let mut keys = BTreeSet::new();
                while let Some(key) = map.next_key::<String>()? {
                    if !keys.insert(key) {
                        return Err(A::Error::custom("Duplicate JSON object key"));
                    }
                    map.next_value::<UniqueKeys>()?;
                }
                Ok(UniqueKeys)
            }
        }
        deserializer.deserialize_any(KeysVisitor)
    }
}

/// Rejects duplicate keys at every depth, including escaped spellings of the same key.
/// The caller must bound the input length; serde's normal recursion limit remains enabled.
pub fn decode_json<T: DeserializeOwned>(bytes: &[u8]) -> serde_json::Result<T> {
    let mut reader = serde_json::Deserializer::from_slice(bytes);
    UniqueKeys::deserialize(&mut reader)?;
    reader.end()?;
    serde_json::from_slice(bytes)
}

/// Reads at most the configuration/package budget plus one byte, even if the file grows.
pub fn read_json<T: DeserializeOwned>(path: &Path) -> crate::Result<T> {
    let storage = |error: std::io::Error| {
        crate::Error::new(crate::ErrorCode::StorageFailure, error.to_string(), "document")
    };
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(storage)?
        .take(crate::MAX_CONFIGURATION_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(storage)?;
    if bytes.len() as u64 > crate::MAX_CONFIGURATION_BYTES {
        return Err(crate::Error::new(
            crate::ErrorCode::InvalidConfiguration,
            "JSON size budget exceeded",
            "document",
        ));
    }
    decode_json(&bytes).map_err(|error| {
        crate::Error::new(crate::ErrorCode::InvalidConfiguration, error.to_string(), "document")
    })
}

#[cfg(test)]
mod tests {
    use super::decode_json;
    #[test]
    fn duplicates_are_rejected_before_maps_or_tagged_values_can_normalize_them() {
        for input in [
            r#"{"roots":{"main":"private","main":"public"}}"#,
            r#"{"parameters":{"gutter":"8","gutter":"80"}}"#,
            r#"{"kind":"snapshot","kind":"recover"}"#,
            r#"[{"scope":[],"\u0073cope":["extra"]}]"#,
            r#"{"windows":{"resource":{},"resource":{}}}"#,
        ] {
            assert!(decode_json::<serde_json::Value>(input.as_bytes()).is_err());
        }
        let input = br#"{"nested":[{"x":true,"n":null,"s":"a","v":-1.5}],"other":{"x":1},"large":18446744073709551615}"#;
        assert!(decode_json::<serde_json::Value>(input).is_ok());
        assert!(decode_json::<serde_json::Value>(b"{} {}").is_err());
        assert!(decode_json::<serde_json::Value>(b"[1e400]").is_err());
    }
}
