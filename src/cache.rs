mod get_or_insert;

use crate::map::LightMap;
use crate::waker_node::WakerNode;
use get_or_insert::{GetOrInsertFuture, GetOrTryInsertFuture};
use hashbrown::hash_map::DefaultHashBuilder;

use std::hash::{BuildHasher, Hash};
use std::sync::Arc;

#[derive(Clone)]
/// A concurrent hashmap that allows for effcient async insertion of values
pub struct LightCache<K, V, S = DefaultHashBuilder> {
    map: Arc<LightMap<K, V, S>>,
}

impl<K, V> LightCache<K, V> {
    pub fn new() -> Self {
        LightCache {
            map: Arc::new(LightMap::new()),
        }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        LightCache {
            map: Arc::new(LightMap::with_capacity(capacity)),
        }
    }
}

impl<K, V, S> LightCache<K, V, S>
where
    K: Eq + Hash + Copy,
    V: Clone + Sync,
    S: BuildHasher,
{
    /// Get or insert the value for the given key
    ///
    /// In the case that a key is being inserted by another thread using this method or [`Self::get_or_try_insert`]
    /// tasks will cooperativley compute the value and notify the other task when the value is inserted.
    /// If a task fails to insert the value, (via panic or error) another task will take over until theyve all tried.
    ///
    /// ### Note
    /// If a call to remove is issued between the time of inserting, and waking up tasks, the other tasks will simply see the empty slot and try again
    pub fn get_or_insert<F, Fut>(&self, key: K, init: F) -> GetOrInsertFuture<K, V, S, F, Fut>
    where
        F: FnOnce() -> Fut + Unpin,
        Fut: std::future::Future<Output = V>,
    {
        let (hash, shard) = self.map.shard(&key).unwrap();

        GetOrInsertFuture::Waiting {
            shard,
            key,
            init: Some(init),
            hash,
            build_hasher: &self.map.build_hasher,
            curr_try: None,
        }
    }

    pub fn get_or_try_insert<F, Fut, Err>(&self, key: K, init: F) -> GetOrTryInsertFuture<K, V, F>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<V, Err>>,
    {
        todo!()
    }

    /// Insert a value directly into the cache
    ///
    /// This function doesn't take into account any pending insertions from [`Self::get_or_insert`] or [`Self::get_or_try_insert`]
    /// and will not wait for them to complete, which means it could be overwritten by another task quickly.
    pub fn insert(&self, key: K, value: V) -> Option<V> {
        // todo: should we hook this up to the waker system?
        // maybe we can kill the task computing the future by writing to the waker node that weve finsihed if its get woken up before the poll finishes
        self.map.insert(key, value)
    }

    /// Try to get a value from the cache
    pub fn get(&self, key: &K) -> Option<V> {
        // todo: should we make this async or introduce a new method to await for pending tasks?
        self.map.get(key)
    }

    /// Remove a value from the cache, returning the value if it was present
    /// 
    /// If this is called while another task is trying to [`Self::get_or_insert`] or [`Self::get_or_try_insert`],
    /// it will force them to recompute the value and insert it again.
    pub fn remove(&self, key: &K) -> Option<V> {
        self.map.remove(key)
    }
}
