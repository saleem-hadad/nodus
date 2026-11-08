//! Fields typed `field?: T | null` in the TypeScript, where leaving the field
//! out and sending `null` mean different things: `None` is absent,
//! `Some(None)` is `null`. Used as
//! `#[serde(default, skip_serializing_if = "Option::is_none", with = "crate::nullable")]`.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub(crate) fn serialize<S, T>(value: &Option<Option<T>>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
    T: Serialize,
{
    match value {
        Some(value) => value.serialize(serializer),
        None => serializer.serialize_none(),
    }
}

pub(crate) fn deserialize<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}
