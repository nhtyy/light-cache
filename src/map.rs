pub mod builder;

use std::hash::{BuildHasher, Hasher};
use std::sync::{Mutex, RwLock};

use hashbrown::hash_map::{DefaultHashBuilder, HashMap};
use hashbrown::raw::RawTable;

use crate::waker_node::Wakers;

/// A concurrent hashmap implementation thats always non-blocking.
///
/// Calls to get and insert are not async and since values are clone, they will never block another thread.
pub struct LightMap<K, V, S = DefaultHashBuilder> {
    pub(crate) build_hasher: S,
    shards: Box<[Shard<K, V>]>,
}

pub struct Shard<K, V> {
    pub(crate) waiters: Mutex<HashMap<K, Wakers>>,
    pub(crate) table: RwLock<RawTable<Entry<K, V>>>,
}

pub struct Entry<K, V> {
    key: K,
    value: V,
}

impl<K, V> LightMap<K, V> {
    pub fn new() -> Self {
        builder::MapBuilder::new().build(Default::default())
    }

    pub fn with_capacity(capacity: usize) -> Self {
        builder::MapBuilder::new()
            .estimated_size(capacity)
            .build(Default::default())
    }
}

impl<K, V, S: BuildHasher> LightMap<K, V, S> {
    pub fn with_hasher(build_hasher: S) -> Self {
        builder::MapBuilder::new().build(build_hasher)
    }

    pub fn with_capacity_and_hasher(capacity: usize, build_hasher: S) -> Self {
        builder::MapBuilder::new()
            .estimated_size(capacity)
            .build(build_hasher)
    }
}

impl<K, V, S> LightMap<K, V, S>
where
    K: Eq + std::hash::Hash,
    S: std::hash::BuildHasher,
    V: Clone,
{
    pub fn insert(&self, key: K, value: V) -> Option<V> {
        let (hash, shard) = self.shard(&key).unwrap();

        shard.insert(key, value, hash, &self.build_hasher)
    }

    pub fn get(&self, key: &K) -> Option<V> {
        let (hash, shard) = self.shard(key).unwrap();

        shard.get(key, hash)
    }

    pub fn remove(&self, key: &K) -> Option<V> {
        let (hash, shard) = self.shard(key).unwrap();

        shard.remove(key, hash)
    }

    pub(crate) fn shard(&self, key: &K) -> Option<(u64, &Shard<K, V>)> {
        let hash = hash_key(&self.build_hasher, key);

        // todo
        let idx = hash as usize % self.shards.len();
        self.shards.get(idx).map(|s| (hash, s))
    }
}

impl<K, V> Shard<K, V>
where
    K: Eq + std::hash::Hash,
    V: Clone,
{
    pub(crate) fn insert<S: BuildHasher>(
        &self,
        key: K,
        value: V,
        hash: u64,
        build_hasher: &S,
    ) -> Option<V> {
        let mut table = self.table.write().expect("table poisoned");

        match table.find_or_find_insert_slot(
            hash,
            |e| eq_key(&key, &e.key),
            |e| hash_key(build_hasher, &e.key),
        ) {
            // saftey: we hold an exclusive lock on the table
            Ok(entry) => unsafe { Some(std::mem::replace(&mut entry.as_mut().value, value)) },
            Err(slot) => {
                let entry = Entry { key, value };
                unsafe {
                    table.insert_in_slot(hash, slot, entry);
                }

                None
            }
        }
    }

    pub(crate) fn get(&self, key: &K, hash: u64) -> Option<V> {
        let table = self.table.read().expect("table poisoned");

        table
            .get(hash, |e| eq_key(key, &e.key))
            .map(|e| e.value.clone())
    }

    pub(crate) fn remove(&self, key: &K, hash: u64) -> Option<V> {
        let mut table = self.table.write().expect("table poisoned");

        table.remove_entry(hash, |e| eq_key(key, &e.key)).map(|e| e.value)
    }
}

pub(crate) fn eq_key<K: Eq>(a: &K, b: &K) -> bool {
    a.eq(b)
}

pub(crate) fn hash_key<K, S>(build_hasher: &S, key: &K) -> u64
where
    K: std::hash::Hash,
    S: std::hash::BuildHasher,
{
    let mut hasher = build_hasher.build_hasher();
    key.hash(&mut hasher);
    hasher.finish()
}

fn max_parrellism() -> usize {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static AVAILABLE_PARALLELISM: AtomicUsize = AtomicUsize::new(0);
    let mut ap = AVAILABLE_PARALLELISM.load(Ordering::Relaxed);
    if ap == 0 {
        ap = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1);

        AVAILABLE_PARALLELISM.store(ap, Ordering::Relaxed);
    }
    ap
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_insert_and_get_basic() {
        let map = LightMap::new();

        map.insert(1, 2);
        map.insert(2, 3);
        map.insert(3, 4);

        assert_eq!(map.get(&1), Some(2));
        assert_eq!(map.get(&2), Some(3));
        assert_eq!(map.get(&3), Some(4));
    }
}