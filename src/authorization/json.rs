use serde::Deserialize;
use serde_json::{Map, Value};

use crate::{AuthError, Result};

// Preserve top-level member uniqueness before converting values into a map.
// Nested extension values remain opaque JSON owned by the caller.
pub(super) struct UniqueJsonObject(pub(super) Map<String, Value>);

impl<'de> Deserialize<'de> for UniqueJsonObject {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        struct ObjectVisitor;

        impl<'de> serde::de::Visitor<'de> for ObjectVisitor {
            type Value = UniqueJsonObject;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a JSON object with unique member names")
            }

            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut entries: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                let mut object = Map::new();
                while let Some(name) = entries.next_key::<String>()? {
                    if object.contains_key(&name) {
                        return Err(serde::de::Error::custom("duplicate response member"));
                    }
                    object.insert(name, entries.next_value::<Value>()?);
                }
                Ok(UniqueJsonObject(object))
            }
        }

        deserializer.deserialize_map(ObjectVisitor)
    }
}

pub(super) fn take_required_string(object: &mut Map<String, Value>, key: &str) -> Result<String> {
    object
        .remove(key)
        .and_then(|value| value.as_str().map(str::to_owned))
        .ok_or(AuthError::InvalidTokenResponse(
            "required string field is missing or invalid",
        ))
}

pub(super) fn take_optional_string(
    object: &mut Map<String, Value>,
    key: &str,
) -> Result<Option<String>> {
    match object.remove(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value)),
        Some(_) => Err(AuthError::InvalidTokenResponse(
            "optional string field has an invalid value",
        )),
    }
}

pub(super) fn take_optional_u64(object: &mut Map<String, Value>, key: &str) -> Result<Option<u64>> {
    match object.remove(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(value)) => {
            value
                .as_u64()
                .map(Some)
                .ok_or(AuthError::InvalidTokenResponse(
                    "expires_in must be a non-negative integer",
                ))
        }
        Some(_) => Err(AuthError::InvalidTokenResponse(
            "expires_in must be a non-negative integer",
        )),
    }
}
