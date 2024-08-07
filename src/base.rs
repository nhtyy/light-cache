mod get_or_insert;
mod waker_node;

use get_or_insert::{GetOrInsertFuture, GetOrTryInsertFuture};
use waker_node::WakerNode;

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::Arc;
use std::sync::{Mutex, RwLock};

#[derive(Clone)]
pub struct LightCache<K, V> {
    inner: Arc<LightCacheInner<K, V>>,
}

pub struct LightCacheInner<K, V> {
    waiters: Mutex<HashMap<K, WakerNode>>,
    map: RwLock<HashMap<K, V>>,
}

impl<K, V> LightCache<K, V> {
    pub fn new() -> Self {
        LightCache {
            inner: Arc::new(LightCacheInner {
                waiters: Mutex::new(HashMap::new()),
                map: RwLock::new(HashMap::new()),
            }),
        }
    }
}

impl<K, V> std::ops::Deref for LightCache<K, V> {
    type Target = LightCacheInner<K, V>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<K, V> LightCache<K, V>
where
    K: Eq + Hash + Copy,
    V: Clone + Sync,
{
    pub fn get_or_insert<F, Fut>(&self, key: K, init: F) -> GetOrInsertFuture<K, V, F, Fut>
    where
        F: FnOnce() -> Fut + Unpin,
        Fut: std::future::Future<Output = V>,
    {
        GetOrInsertFuture::Waiting {
            cache: self,
            key,
            init: Some(init),
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

    pub fn insert(&self, key: K, value: V) -> Option<V> {
        // insert directly into the map ignoring another other writers (which may very well subsequently override this value)
        self.map.write().expect("rw lock poisoned").insert(key, value)
    }

    pub fn get(&self, key: K) -> Option<V> {
        self.map.read().expect("rw lock poisoned").get(&key).cloned()
    }

    pub fn remove(&self, key: K) -> Option<V> {
        self.map.write().expect("rw lock poisoned").remove(&key)
    }
}