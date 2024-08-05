use tokio::sync::RwLock;

use std::borrow::Borrow;
use std::collections::hash_map::Entry;
use std::collections::hash_map::OccupiedEntry;
use std::collections::hash_map::VacantEntry;
use std::collections::HashMap;
use std::future::Future;
use std::hash::Hash;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;
use std::task::ready;
use std::task::Context;
use std::task::Poll;
use std::task::Waker;

struct LightCache<K, V> {
    waiters: Mutex<HashMap<K, Vec<Waker>>>,
    map: RwLock<HashMap<K, Arc<V>>>,
}

impl<K, V> LightCache<K, V>
where
    K: Eq + Hash + Copy,
    V: Sync,
{
    fn new() -> Self {
        LightCache {
            waiters: Mutex::new(HashMap::new()),
            map: RwLock::new(HashMap::new()),
        }
    }

    async fn get_or_insert<Q, F, Fut>(&self, key: K, init: F) -> Arc<V>
    where
        Q: Borrow<K>,
        F: FnOnce() -> Fut + Unpin,
        Fut: std::future::Future<Output = V>,
    {
        if let Some(value) = self.map.read().await.get(key.borrow()) {
            return value.clone();
        }

        GetOrInsertFuture::new(self, key, init).await
    }

    async fn get_or_try_insert<Q, F, Fut, Err>(&self, key: K, init: F) -> Result<Arc<V>, Err>
    where
        Q: Borrow<K>,
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<V, Err>>,
    {
        if let Some(value) = self.map.read().await.get(key.borrow()) {
            return Ok(value.clone());
        }

        GetOrTryInsertFuture::new(self, key, init).await
    }
}

struct GetOrInsertFuture<'a, K, V, F, Fut> {
    cache: &'a LightCache<K, V>,
    key: K,
    init: Option<F>,
    fut: Option<Fut>,
}

impl<'a, K, V, F, Fut> GetOrInsertFuture<'a, K, V, F, Fut> {
    fn new(cache: &'a LightCache<K, V>, key: K, init: F) -> Self {
        GetOrInsertFuture {
            cache,
            key,
            init: Some(init),
            fut: None,
        }
    }
}

impl<'a, K, V, F, Fut> Future for GetOrInsertFuture<'a, K, V, F, Fut>
where
    K: Eq + Hash + Copy,
    V: Sync,
    F: FnOnce() -> Fut + Unpin,
    Fut: std::future::Future<Output = V>,
{
    type Output = Arc<V>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // check cache again at top

        // saftey: F is unpin
        // saftey: fut is never moved
        let this = unsafe { self.get_unchecked_mut() };

        if let Some(fut) = this.fut.as_mut() {
            ready!(unsafe { Pin::new_unchecked(fut) }.poll(cx));
        }

        match this
            .cache
            .waiters
            .lock()
            .expect("lock not poisoned")
            .entry(this.key)
        {
            Entry::Occupied(mut v) => {
                v.get_mut().push(cx.waker().clone());

                return Poll::Pending;
            },
            Entry::Vacant(v) => {
                v.insert(Vec::new());

                // saftey: F is unpin
                this.fut = Some(this.init.take().expect("init is not None")());
            }
        }

        todo!()
    }
}

struct GetOrTryInsertFuture<'a, K, V, F> {
    cache: &'a LightCache<K, V>,
    key: K,
    init: F,
}

impl<'a, K, V, F> GetOrTryInsertFuture<'a, K, V, F> {
    fn new(cache: &'a LightCache<K, V>, key: K, init: F) -> Self {
        GetOrTryInsertFuture { cache, key, init }
    }
}

impl<'a, K, V, F, Fut, E> Future for GetOrTryInsertFuture<'a, K, V, F>
where
    K: Eq + Hash + Copy,
    V: Sync,
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<V, E>>,
{
    type Output = Result<Arc<V>, E>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        todo!()
    }
}
// construct a goifut in the goi funct and then await it
