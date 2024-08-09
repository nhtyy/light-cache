mod get_or_insert;

use crate::map::LightMap;
use crate::policy::{NoopPolicy, Policy};
use get_or_insert::{GetOrInsertFuture, GetOrTryInsertFuture};
use hashbrown::hash_map::DefaultHashBuilder;

use std::hash::{BuildHasher, Hash};
use std::ops::Deref;
use std::sync::Arc;

/// A concurrent hashmap that allows for effcient async insertion of values
pub struct LightCache<K, V, S = DefaultHashBuilder, P = NoopPolicy> {
    inner: Arc<LightCacheInner<K, V, S, P>>,
}

impl<K, V, S, P> Clone for LightCache<K, V, S, P> {
    fn clone(&self) -> Self {
        LightCache {
            inner: self.inner.clone(),
        }
    }
}

impl<K, V, S, P> Deref for LightCache<K, V, S, P> {
    type Target = LightCacheInner<K, V, S, P>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

pub struct LightCacheInner<K, V, S = DefaultHashBuilder, P = NoopPolicy> {
    pub(crate) policy: P,
    map: LightMap<K, V, S>,
}

impl<K, V> LightCache<K, V> {
    pub fn new() -> Self {
        LightCache {
            inner: Arc::new(LightCacheInner {
                map: LightMap::new(),
                policy: NoopPolicy,
            }),
        }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        LightCache {
            inner: Arc::new(LightCacheInner {
                map: LightMap::with_capacity(capacity),
                policy: NoopPolicy,
            }),
        }
    }
}

impl<K, V, S: BuildHasher, P> LightCache<K, V, S, P> {
    pub fn from_parts(policy: P, hasher: S) -> Self {
        LightCache {
            inner: Arc::new(LightCacheInner {
                map: LightMap::with_hasher(hasher),
                policy,
            }),
        }
    }

    pub fn from_parts_with_capacity(policy: P, hasher: S, capacity: usize) -> Self {
        LightCache {
            inner: Arc::new(LightCacheInner {
                map: LightMap::with_capacity_and_hasher(capacity, hasher),
                policy,
            }),
        }
    } 
}

impl<K, V, S, P> LightCache<K, V, S, P> {
    pub fn len(&self) -> usize {
        self.map.len()
    }
}

impl<K, V, S, P> LightCache<K, V, S, P>
where
    K: Eq + Hash + Copy,
    V: Clone + Sync,
    S: BuildHasher,
    P: Policy<K, V>,
{
    /// Get or insert the value for the given key
    ///
    /// In the case that a key is being inserted by another thread using this method or [`Self::get_or_try_insert`]
    /// tasks will cooperativley compute the value and notify the other task when the value is inserted.
    /// If a task fails to insert the value, (via panic or error) another task will take over until theyve all tried.
    ///
    /// ### Note
    /// If a call to remove is issued between the time of inserting, and waking up tasks, the other tasks will simply see the empty slot and try again
    pub async fn get_or_insert<F, Fut>(&self, key: K, init: F) -> V
    where
        F: FnOnce() -> Fut + Unpin,
        Fut: std::future::Future<Output = V>,
    {
        let inner = self.get_or_insert_inner(key, init).await;
        self.policy.after_get_or_insert(&key, self);

        inner
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
        // todo: should we hook this up to the waitiers?
        // maybe we can kill the task computing the future by writing to the waker node that weve finsihed if its get woken up before the poll finishes
        let v = self.map.insert(key, value);
        self.policy.after_get_or_insert(&key, self);

        v
    }

    /// Try to get a value from the cache
    pub fn get(&self, key: &K) -> Option<V> {
        // todo: should we make this async or introduce a new method to await for pending tasks?
        self.map.get(key).and_then(|v| {
            self.policy.after_get_or_insert(key, self);
            Some(v)
        })
    }

    /// Remove a value from the cache, returning the value if it was present
    ///
    /// If this is called while another task is trying to [`Self::get_or_insert`] or [`Self::get_or_try_insert`],
    /// it will force them to recompute the value and insert it again.
    pub fn remove(&self, key: &K) -> Option<V> {
        self.map.remove(key).and_then(|v| {
            self.policy.after_remove(key, self);
            Some(v)
        })
    }

    pub(crate) fn remove_no_policy(&self, key: &K) -> Option<V> {
        self.map.remove(key)
    }
}

impl<K, V, S, P> LightCache<K, V, S, P>
where
    K: Eq + Hash + Copy,
    V: Clone + Sync,
    S: BuildHasher,
    P: Policy<K, V>,
{
    fn get_or_insert_inner<F, Fut>(&self, key: K, init: F) -> GetOrInsertFuture<K, V, S, F, Fut> {
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
}
