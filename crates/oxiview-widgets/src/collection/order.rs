use super::CollectionError;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Default)]
pub(super) struct KeyOrder {
    pub keys: Vec<u64>,
    pub indices: HashMap<u64, usize>,
}

impl KeyOrder {
    pub fn new(keys: impl IntoIterator<Item = u64>) -> Result<Self, CollectionError> {
        let keys: Vec<_> = keys.into_iter().collect();
        let mut indices = HashMap::with_capacity(keys.len());
        for (index, &key) in keys.iter().enumerate() {
            if indices.insert(key, index).is_some() {
                return Err(CollectionError::DuplicateKey(key));
            }
        }
        Ok(Self { keys, indices })
    }

    pub fn checked_tail(
        &self,
        keys: impl IntoIterator<Item = u64>,
    ) -> Result<Vec<u64>, CollectionError> {
        let keys: Vec<_> = keys.into_iter().collect();
        let mut seen = HashSet::with_capacity(keys.len());
        for &key in &keys {
            if self.indices.contains_key(&key) || !seen.insert(key) {
                return Err(CollectionError::DuplicateKey(key));
            }
        }
        Ok(keys)
    }

    pub fn append(&mut self, keys: Vec<u64>) {
        let first = self.keys.len();
        self.indices.extend(
            keys.iter()
                .enumerate()
                .map(|(index, &key)| (key, first + index)),
        );
        self.keys.extend(keys);
    }

    pub fn index(&self, key: u64) -> Option<usize> {
        self.indices.get(&key).copied()
    }

    pub fn key(&self, index: usize) -> Option<u64> {
        self.keys.get(index).copied()
    }

    pub fn between(&self, a: u64, b: u64) -> Option<&[u64]> {
        let a = self.index(a)?;
        let b = self.index(b)?;
        self.keys.get(a.min(b)..=a.max(b))
    }
}
