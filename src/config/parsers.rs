//! Custom parsers for configurations.

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
pub fn polymorphic_vec<'de, D, T, C>(deserializer: D) -> Result<C, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + FromStr + TryFromKv,
    C: From<Vec<T>>,
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

    let vec = match Container::<T>::deserialize(deserializer)? {
        Container::List(items) => items
            .into_iter()
            .map(|item| match item {
                Item::Obj(val) => Ok(val),
                Item::Str(s) => s.parse().map_err(serde::de::Error::custom),
            })
            .collect::<Result<Vec<T>, _>>()?,
        Container::Map(map) => map
            .into_iter()
            .map(|(k, v)| T::try_from_kv(k, v).map_err(serde::de::Error::custom))
            .collect::<Result<Vec<T>, _>>()?,
    };

    Ok(C::from(vec))
}

/// Overwrites the base vector with the top vector if the top vector is not empty.
pub fn vec_replace<T>(base: Vec<T>, top: Vec<T>) -> Vec<T> {
    if top.is_empty() { base } else { top }
}

/// Appends items from `top` to `base`, regardless of duplicates.
pub fn vec_extend<T>(mut base: Vec<T>, top: Vec<T>) -> Vec<T> {
    base.extend(top);
    base
}

/// Appends items from `top` to `base` if they are not already present in `base`.
pub fn vec_dedup<T: PartialEq>(mut base: Vec<T>, top: Vec<T>) -> Vec<T> {
    for item in top {
        if !base.contains(&item) {
            base.push(item);
        }
    }
    base
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::path::PathMapping;
    use serde::Deserialize;
    #[derive(Deserialize)]

    struct Config {
        #[serde(deserialize_with = "polymorphic_vec")]
        map: Vec<PathMapping>,
    }

    #[test]
    fn test_vec_replace() {
        let base = vec!["a", "b"];
        let top = vec!["c"];
        let empty: Vec<&str> = vec![];

        assert_eq!(vec_replace(base.clone(), top.clone()), vec!["c"]);
        assert_eq!(vec_replace(base.clone(), empty), vec!["a", "b"]);
    }

    #[test]
    fn test_vec_dedup() {
        let base = vec![1, 2, 3];
        let top = vec![3, 4, 5];

        let merged = vec_dedup(base, top);

        assert_eq!(merged, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn test_vec_dedup_base_duplicates() {
        // vec_dedup does NOT clean base. It only prevents adding *new* duplicates from top.
        let base = vec![1, 1, 2];
        let top = vec![2, 3];

        let merged = vec_dedup(base, top);
        assert_eq!(merged, vec![1, 1, 2, 3]);
    }

    #[test]
    fn test_path_mapping_polymorphism() {
        let source_file = tempfile::NamedTempFile::new().unwrap();
        let src_path = source_file.path().to_str().unwrap();

        let toml_input = format!(
            r#"
                map = [
                    "{src}:/tmp/dst1",
                    {{ src = "{src}", dst = "/tmp/dst2" }}
                ]
                "#,
            src = src_path
        );

        let config: Config = toml::from_str(&toml_input).expect("Parsing failed");

        assert_eq!(config.map.len(), 2);

        assert_eq!(config.map[0].src().as_path(), source_file.path());
        assert_eq!(config.map[1].src().as_path(), source_file.path());
    }
}
