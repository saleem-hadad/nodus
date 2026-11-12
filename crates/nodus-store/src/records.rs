//! Records a board keeps by id, read leniently. One this version can't read
//! (a newer version's kind of shape, say) is kept as it was and written
//! back, rather than making state.json unreadable or being lost on save.

use indexmap::IndexMap;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
enum Record<T> {
    Known(T),
    Kept(Value),
}

/// A board's records by id, in the order they were written.
#[derive(Debug, Clone, PartialEq)]
pub struct Records<T> {
    entries: IndexMap<String, Record<T>>,
}

impl<T> Default for Records<T> {
    fn default() -> Self {
        Self {
            entries: IndexMap::new(),
        }
    }
}

impl<T> Records<T> {
    /// Nothing at all, read or kept.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The records this version can read, in order.
    pub fn values(&self) -> impl Iterator<Item = &T> {
        self.entries.values().filter_map(|r| match r {
            Record::Known(t) => Some(t),
            Record::Kept(_) => None,
        })
    }

    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut T> {
        self.entries.values_mut().filter_map(|r| match r {
            Record::Known(t) => Some(t),
            Record::Kept(_) => None,
        })
    }

    pub fn get(&self, id: &str) -> Option<&T> {
        match self.entries.get(id)? {
            Record::Known(t) => Some(t),
            Record::Kept(_) => None,
        }
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut T> {
        match self.entries.get_mut(id)? {
            Record::Known(t) => Some(t),
            Record::Kept(_) => None,
        }
    }

    /// Whether a record has this id, one this version reads or not.
    pub fn contains_key(&self, id: &str) -> bool {
        self.entries.contains_key(id)
    }

    /// Puts a record in, in place of one with its id (keeping its place).
    pub fn insert(&mut self, id: String, value: T) {
        self.entries.insert(id, Record::Known(value));
    }

    pub fn shift_remove(&mut self, id: &str) -> Option<T> {
        match self.entries.shift_remove(id)? {
            Record::Known(t) => Some(t),
            Record::Kept(_) => None,
        }
    }

    /// Keeps the records `keep` says to, and every one this version can't read.
    pub fn retain(&mut self, mut keep: impl FnMut(&T) -> bool) {
        self.entries.retain(|_, r| match r {
            Record::Known(t) => keep(t),
            Record::Kept(_) => true,
        });
    }
}

impl<T: Serialize> Serialize for Records<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(Some(self.entries.len()))?;
        for (id, record) in &self.entries {
            match record {
                Record::Known(t) => map.serialize_entry(id, t)?,
                Record::Kept(v) => map.serialize_entry(id, v)?,
            }
        }
        map.end()
    }
}

impl<'de, T: DeserializeOwned> Deserialize<'de> for Records<T> {
    /// Anything but an object is no records.
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let Value::Object(raw) = Value::deserialize(deserializer)? else {
            return Ok(Self::default());
        };
        let entries = raw
            .into_iter()
            .map(|(id, value)| {
                let record = match serde_json::from_value::<T>(value.clone()) {
                    Ok(t) => Record::Known(t),
                    Err(_) => Record::Kept(value),
                };
                (id, record)
            })
            .collect();
        Ok(Self { entries })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct Thing {
        kind: String,
        n: u32,
    }

    #[test]
    fn keeps_what_it_cannot_read_and_writes_it_back() {
        let raw = json!({ "a": { "kind": "box", "n": 1 }, "b": { "kind": "star", "n": "many" } });
        let mut records: Records<Thing> = serde_json::from_value(raw.clone()).unwrap();
        assert_eq!(records.values().count(), 1);
        assert!(records.contains_key("b"));
        assert_eq!(records.get("b"), None);
        assert_eq!(serde_json::to_value(&records).unwrap(), raw);
        records.retain(|_| false);
        assert_eq!(
            serde_json::to_value(&records).unwrap(),
            json!({ "b": { "kind": "star", "n": "many" } })
        );
    }

    #[test]
    fn reads_anything_but_an_object_as_none() {
        for raw in [json!([]), json!(null), json!("x")] {
            assert!(serde_json::from_value::<Records<Thing>>(raw).unwrap().is_empty());
        }
    }
}
