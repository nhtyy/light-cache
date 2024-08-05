use tokio::sync::RwLock;

use std::hash::Hash;
use std::borrow::Borrow;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::task::Waker;

struct LightCache<K, V> {
    waiters: Mutex<HashMap<K, Waker>>,
    map: RwLock<HashMap<K, Arc<V>>>,
}

impl<K, V> LightCache<K, V>
where
    K: Eq + Hash,
    V: Sync,
{
    fn new() -> Self {
        LightCache {
            waiters: Mutex::new(HashMap::new()),
            map: RwLock::new(HashMap::new()),
        }
    }

    async fn get_or_insert<Q, F, Fut>(&self, key: Q, init: F) -> Arc<V>
    where
        Q: Borrow<K>,
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = V>,
    {
        if let Some(value) = self.map.read().await.get(key.borrow()) {
            return value.clone();
        }

        todo!()
    }

    async fn get_or_try_insert<Q, F, Fut, Err>(&self, key: Q, init: F) -> Result<Arc<V>, Err>
    where
        Q: Borrow<K>,
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<V, Err>>,
    {
        if let Some(value) = self.map.read().await.get(key.borrow()) {
            return Ok(value.clone());
        }

        todo!()
    }
}

struct GetOrInsertFuture<'a, K, V, Q, F> {
    cache: &'a LightCache<K, V>,
    key: Q,
    init: F,
}

impl<'a, K, V, Q, F> GetOrInsertFuture<'a, K, V, Q, F> {
    fn new(cache: &'a LightCache<K, V>, key: Q, init: F) -> Self {
        GetOrInsertFuture { cache, key, init }
    }
}

struct GetOrTryInsertFuture<'a, K, V, Q, F> {
    cache: &'a LightCache<K, V>,
    key: Q,
    init: F,
}

impl<'a, K, V, Q, F> GetOrTryInsertFuture<'a, K, V, Q, F> {
    fn new(cache: &'a LightCache<K, V>, key: Q, init: F) -> Self {
        GetOrTryInsertFuture { cache, key, init }
    }
}
