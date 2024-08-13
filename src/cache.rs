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
    /// ## Note
    /// If a call to remove is issued between the time of inserting, and waking up tasks, the other tasks will simply see the empty slot and try again
    pub async fn get_or_insert<F, Fut>(&self, key: K, init: F) -> V
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = V>,
    {
        if let Some(value) = self.get(&key) {
            return value;
        }

        self.get_or_insert_inner(key, init).await
    }

    /// Get or insert the value for the given key, returning an error if the value could not be inserted
    /// 
    /// See [`Self::get_or_insert`] for more information
    pub async fn get_or_try_insert<F, Fut, Err>(
        &self,
        key: K,
        init: F,
    ) -> Result<V, Err>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<V, Err>>,
    {
        if let Some(value) = self.get(&key) {
            return Ok(value);
        }

        self.get_or_try_insert_inner(key, init).await
    }

    /// Get or insert a value into the cache, but instead of waiting for a single caller to finish the insertion (coopertively),
    /// any callers of this method will always attempt to poll thier own future
    /// 
    /// This is safe to use with and other get_or_* method
    pub async fn get_or_insert_race<F, Fut>(&self, key: K, init: F) -> V
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = V>,
    {
        if let Some(val) = self.get(&key) {
            return val;
        }

        self.get_or_insert_race_inner(key, init).await
    }

    /// See [`Self::get_or_insert_race`] for more information
    pub async fn get_or_try_insert_race<F, Fut, E>(&self, key: K, init: F) -> Result<V, E>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<V, E>>,
    {
        if let Some(val) = self.get(&key) {
            return Ok(val);
        }

        self.get_or_try_insert_race_inner(key, init).await
    }

    /// Insert a value directly into the cache
    /// 
    /// ## Warning: 
    /// Doing a [`Self::insert`] while another task is doing any type of async write ([Self::get_or_insert], [Self::get_or_try_insert], etc)
    /// will "leak" a [`crate::map::Wakers`] entry, which will never be removed unless another `get_or_*` method is fully completed without being
    /// interrupted by a call to this method.
    /// 
    /// This is mostly not a big deal as Wakers is small, and this pattern really should be avoided in practice.
    #[inline]
    pub fn insert(&self, key: K, value: V) -> Option<V> {
        self.policy.insert(key, value, self)
    }

    /// Try to get a value from the cache
    #[inline]
    pub fn get(&self, key: &K) -> Option<V> {
        self.policy.get(key, self)
    }

    /// Remove a value from the cache, returning the value if it was present
    ///
    /// ## Note: 
    /// If this is called while another task is trying to [`Self::get_or_insert`] or [`Self::get_or_try_insert`],
    /// it will force them to recompute the value and insert it again.
    #[inline]
    pub fn remove(&self, key: &K) -> Option<V> {
        self.policy.remove(key, self)
    }

    /// Prune the cache of any expired keys
    #[inline]
    pub fn prune(&self) {
        self.policy.prune(self)
    }

    #[inline]
    pub(crate) fn get_no_policy(&self, key: &K) -> Option<V> {
        self.map.get(key)
    }

    #[inline]
    pub(crate) fn insert_no_policy(&self, key: K, value: V) -> Option<V> {
        self.map.insert(key, value)
    }

    #[inline]
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
    #[inline]
    fn get_or_insert_inner<F, Fut>(&self, key: K, init: F) -> GetOrInsertFuture<K, V, S, P, F, Fut>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = V>,
    {
        let (hash, shard) = self.map.shard(&key).unwrap();

        GetOrInsertFuture::new(self, shard, hash, key, init)
    }

    #[inline]
    fn get_or_try_insert_inner<F, Fut, E>(
        &self,
        key: K,
        init: F,
    ) -> GetOrTryInsertFuture<K, V, S, P, F, Fut>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<V, E>>,
    {
        let (hash, shard) = self.map.shard(&key).unwrap();

        GetOrTryInsertFuture::new(self, shard, hash, key, init)
    }

    #[inline]
    fn get_or_insert_race_inner<F, Fut>(&self, key: K, init: F) -> GetOrInsertRace<K, V, S, P, Fut>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = V>,
    {
        let (hash, shard) = self.map.shard(&key).unwrap();

        GetOrInsertRace::new(self, shard, hash, key, init())
    }

    #[inline]
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
}
