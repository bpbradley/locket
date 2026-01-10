use serde::{Deserialize, Deserializer};
use std::collections::HashMap;
use std::fmt::Display;
use std::str::FromStr;

/// Trait for types that can be inflated from KV
pub trait TryFromKv: Sized {
    type Err: Display;
    fn try_from_kv(key: String, val: String) -> Result<Self, Self::Err>;
}

/// deserializes a list or a map into Vec<T>.
pub fn polymorphic_vec<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + FromStr + TryFromKv,
    <T as FromStr>::Err: Display,
    <T as TryFromKv>::Err: Display,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Item<T> {
        Str(String),
        Obj(T),
    }

    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Container<T> {
        List(Vec<Item<T>>),
        Map(HashMap<String, String>),
    }

    match Container::<T>::deserialize(deserializer)? {
        Container::List(items) => items
            .into_iter()
            .map(|item| match item {
                Item::Obj(val) => Ok(val),
                Item::Str(s) => s.parse().map_err(serde::de::Error::custom),
            })
            .collect(),
        Container::Map(map) => map
            .into_iter()
            .map(|(k, v)| T::try_from_kv(k, v).map_err(serde::de::Error::custom))
            .collect(),
    }
}
