pub(crate) mod get_or_insert;
pub(crate) use get_or_insert::{GetOrInsertFuture, GetOrTryInsertFuture};

pub(crate) mod get_or_insert_race;
pub(crate) use get_or_insert_race::{GetOrInsertRace, GetOrTryInsertRace};

use crate::map::LightMap;
use crate::policy::{NoopPolicy, Policy};
pub use hashbrown::hash_map::DefaultHashBuilder;

use std::future::Future;
use std::hash::{BuildHasher, Hash};
use std::ops::Deref;
use std::sync::Arc;

/// A concurrent hashmap that allows for efficient async insertion of values
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
    pub fn get_or_insert<F, Fut>(&self, key: K, init: F) -> GetOrInsertFuture<K, V, S, P, F, Fut>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = V>,
    {
        if let Some(value) = self.get(&key) {
            return GetOrInsertFuture::Ready(Some(value));
        }

        let (hash, shard) = self.map.shard(&key).unwrap();

        GetOrInsertFuture::Waiting {
            cache: self,
            shard,
            key,
            init: Some(init),
            hash,
            joined: false,
        }
    }

    pub fn get_or_try_insert<F, Fut, Err>(&self, key: K, init: F) -> GetOrTryInsertFuture<K, V, S, P, F, Fut>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<V, Err>>,
    {
        if let Some(value) = self.get(&key) {
            return GetOrTryInsertFuture::Ready(Some(value));
        }

        let (hash, shard) = self.map.shard(&key).unwrap();

        GetOrTryInsertFuture::Waiting {
            cache: self,
            shard,
            key,
            init: Some(init),
            hash,
            joined: false,
        }
    }

    #[inline]
    pub async fn get_or_insert_race<F, Fut>(
        &self,
        key: K,
        init: F,
    ) -> V
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = V>,
    {
        if let Some(val) = self.get(&key) {
            return val;
        }

        self.get_or_insert_race_inner(key, init).await
    }

    pub async fn get_or_try_insert_race<F, Fut, E>(
        &self,
        key: K,
        init: F,
    ) -> Result<V, E>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<V, E>>,
    {
        if let Some(val) = self.get(&key) {
            return Ok(val);
        }

        self.get_or_try_insert_race_inner(key, init).await
    }

    #[inline]
    fn get_or_insert_race_inner<F, Fut>(
        &self,
        key: K,
        init: F,
    ) -> GetOrInsertRace<K, V, S, P, Fut>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = V>,
    {
        let (hash, shard) = self.map.shard(&key).unwrap();

        GetOrInsertRace::new(self, shard, hash, key, init())
    }

    fn get_or_try_insert_race_inner<F, Fut, E>(
        &self,
        key: K,
        init: F,
    ) -> GetOrTryInsertRace<K, V, S, P, Fut>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<V, E>>,
    {
        let (hash, shard) = self.map.shard(&key).unwrap();

        GetOrTryInsertRace::new(self, shard, hash, key, init())
    }

    /// Insert a value directly into the cache
    ///
    /// This function doesn't take into account any pending insertions from [`Self::get_or_insert`] or [`Self::get_or_try_insert`]
    /// and will not wait for them to complete, which means it could be overwritten by another task quickly.
    pub fn insert(&self, key: K, value: V) {
        self.policy.insert(key, value, self)
    }

    /// Try to get a value from the cache
    pub fn get(&self, key: &K) -> Option<V> {
        self.policy.get(key, self)
    }

    /// Remove a value from the cache, returning the value if it was present
    ///
    /// If this is called while another task is trying to [`Self::get_or_insert`] or [`Self::get_or_try_insert`],
    /// it will force them to recompute the value and insert it again.
    pub fn remove(&self, key: &K) -> Option<V> {
        self.policy.remove(key, self)
    }

    #[inline]
    pub(crate) fn get_no_policy(&self, key: &K) -> Option<V> {
        self.map.get(key)
    }

    #[inline]
    pub(crate) fn insert_no_policy(&self, key: K, value: V) {
        self.map.insert(key, value);
    }

    #[inline]
    pub(crate) fn remove_no_policy(&self, key: &K) -> Option<V> {
        self.map.remove(key)
    }
}
