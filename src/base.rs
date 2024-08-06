mod get_or_insert;

use get_or_insert::{GetOrInsertFuture, GetOrTryInsertFuture};

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::Arc;
use std::sync::{Mutex, RwLock};
use std::task::Waker;

#[derive(Clone)]
pub struct LightCache<K, V> {
    inner: Arc<LightCacheInner<K, V>>,
}

pub struct LightCacheInner<K, V> {
    waiters: Mutex<HashMap<K, WakerNode>>,
    map: RwLock<HashMap<K, Arc<V>>>,
}

struct WakerNode {
    curr_try: usize,
    active: bool,
    wakers: Vec<Waker>,
}

impl WakerNode {
    fn start() -> Self {
        WakerNode {
            curr_try: 0,
            active: true,
            wakers: Vec::new(),
        }
    }

    fn attempt(&mut self) -> usize {
        self.curr_try
    }

    fn is_active(&self) -> bool {
        self.active
    }

    fn halt(&mut self) -> Vec<Waker> {
        self.active = false;

        std::mem::take(&mut self.wakers)
    }

    fn activate(&mut self) -> usize {
        self.curr_try += 1;
        self.active = true;

        self.curr_try
    }
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
    V: Sync,
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

    pub fn insert(&self, key: K, value: V) -> Option<Arc<V>> {
        let value = Arc::new(value);

        // insert directly into the map ignoring another other writers (which may very well subsequently override this value)
        self.map.write().expect("rw lock poisoned").insert(key, value)
    }

    pub fn get(&self, key: K) -> Option<Arc<V>> {
        self.map.read().expect("rw lock poisoned").get(&key).cloned()
    }

    pub fn remove(&self, key: K) -> Option<Arc<V>> {
        self.map.write().expect("rw lock poisoned").remove(&key)
    }
}