use std::future::Future;
use std::hash::Hash;
use std::pin::Pin;
use std::sync::Arc;
use std::task::Context;
use std::task::Poll;

use super::LightCache;
use super::WakerNode;

use pin_project::pinned_drop;

#[must_use = "futures do nothing unless you `.await` or poll them"]
#[pin_project::pin_project(project = GetOrInsertFutureProj, PinnedDrop)]
pub enum GetOrInsertFuture<'a, K, V, F, Fut>
where
    K: Eq + Hash,
{
    Waiting {
        cache: &'a LightCache<K, V>,
        key: K,
        init: Option<F>,
        curr_try: Option<usize>,
    },
    Working {
        cache: &'a LightCache<K, V>,
        key: K,
        #[pin]
        fut: Fut,
    },
}

#[pinned_drop]
impl<'a, K, V, F, Fut> PinnedDrop for GetOrInsertFuture<'a, K, V, F, Fut>
where
    K: Eq + Hash,
{
    fn drop(self: Pin<&mut Self>) {
        let project = self.project();

        match project {
            GetOrInsertFutureProj::Working { cache, key, .. } => {
                let mut waiters = cache.waiters.lock().expect("on drop waiters lock not poisoned");

                // notify the wakers and set the node to inactive so another thread can try to insert
                // note: in the happy path the waiters have been removed, so this may be a None
                if let Some(node) = waiters.get_mut(key) {
                    for waker in node.halt() {
                        waker.wake();
                    }
                }
            }
            GetOrInsertFutureProj::Waiting { .. } => {}
        }
    }
}

impl<'a, K, V, F, Fut> Future for GetOrInsertFuture<'a, K, V, F, Fut>
where
    K: Eq + Hash + Copy,
    V: Sync,
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = V>,
{
    type Output = Arc<V>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        loop {
            match self.as_mut().project() {
                GetOrInsertFutureProj::Waiting {
                    cache,
                    key,
                    init,
                    curr_try,
                } => {
                    // do an initial read to see if the value is in the cache, if it is we can return it
                    // we dont wanna lock the mutex here cause then the reads would be serial
                    if let Some(value) = cache.map.read().expect("rw lock poisioned").get(&key) {
                        return Poll::Ready(value.clone());
                    }

                    // the value wasnt in the cache, so lets take the lock so no one else is gonna insert it
                    // and we can insert it
                    let mut waiters_lock = cache.waiters.lock().expect("waiters lock not poisoned");
                    if let Some(value) = cache.map.read().expect("rw lock poisioned").get(&key) {
                        return Poll::Ready(value.clone());
                    }

                    // the value is not in the cache
                    // lets check if there are any waiters if not we need to compute this ourselves
                    // we dont call `init` just yet as it could panic, so lets drop our locks first.
                    match waiters_lock.get_mut(&key) {
                        Some(node) => {
                            if node.is_active() {
                                if let Some(curr_try) = curr_try {
                                    if node.attempt() == curr_try {
                                        return Poll::Pending;
                                    }
                                }

                                // either this is our first attempt and someone is contesting us
                                // or the try number has changed and we need to give them our waker again
                                node.join(cx.waker().clone());
                                curr_try.replace(*node.attempt());

                                return Poll::Pending;
                            } else {
                                curr_try.replace(node.activate());
                            }
                        }
                        None => {
                            waiters_lock.insert(*key, WakerNode::start());
                        }
                    };
                    // at this point we havent early returned, so that means this task is going to try to insert the value
                    drop(waiters_lock);

                    let working = GetOrInsertFuture::Working {
                        cache,
                        key: *key,
                        fut: init.take().expect("init is none")(),
                    };

                    // saftey: we are moving to the next stage
                    let _ =
                        std::mem::replace(unsafe { self.as_mut().get_unchecked_mut() }, working);
                }
                GetOrInsertFutureProj::Working { cache, key, fut } => {
                    if let Poll::Ready(val) = fut.poll(cx) {
                        let val = Arc::new(val);

                        cache
                            .map
                            .write()
                            .expect("rw lock poisoned")
                            .insert(*key, val.clone());

                        let waiters = cache
                            .waiters
                            .lock()
                            .expect("waiters lock not poisoned")
                            .remove(&key);

                        if let Some(node) = waiters {
                            for waker in node.wakers {
                                waker.wake();
                            }
                        } else {
                            // saftey: we always insert an empty array for the key and we should be the only one to remove it
                            unreachable!("no wakers for key, this is a bug");
                        }

                        return Poll::Ready(val);
                    } else {
                        return Poll::Pending;
                    }
                }
            }
        }
    }
}

pub struct GetOrTryInsertFuture<'a, K, V, F> {
    cache: &'a LightCache<K, V>,
    key: K,
    init: F,
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

#[cfg(test)]
mod test {
    #[tokio::test]
    async fn test_get_or_insert_single_caller() {
        use super::super::LightCache;

        let cache = LightCache::new();

        let key = 1;
        let val = cache.get_or_insert(key, || async { 1 }).await;

        assert_eq!(*val, 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_get_or_insert_many_callers_returns_the_same_value_join() {
        use super::super::LightCache;

        let cache = LightCache::new();

        let key = 1;

        let fut1 = cache.get_or_insert(key, || async { 1 });
        let fut2 = cache.get_or_insert(key, || async { 2 });
        let fut3 = cache.get_or_insert(key, || async { 3 });

        let (get1, get2, get3) = tokio::join!(fut1, fut2, fut3);

        assert_eq!(*get1, 1);
        assert_eq!(*get2, 1);
        assert_eq!(*get3, 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_get_or_insert_many_callers_returns_the_same_value_spawn() {
        use super::super::LightCache;

        let cache = LightCache::new();

        let key = 1;

        let get1 = tokio::spawn({
            let cache = cache.clone();
            async move {
                // no sleep
                cache.get_or_insert(key, || async { 1 }).await
            }
        });
        let get2 = tokio::spawn({
            let cache = cache.clone();
            async move {
                cache
                    .get_or_insert(key, || async {
                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                        2
                    })
                    .await
            }
        });
        let get3 = tokio::spawn({
            let cache = cache.clone();
            async move {
                cache
                    .get_or_insert(key, || async {
                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                        3
                    })
                    .await
            }
        });

        let (get1, get2, get3) = tokio::join!(get1, get2, get3);

        assert_eq!(*get1.unwrap(), 1);
        assert_eq!(*get2.unwrap(), 1);
        assert_eq!(*get3.unwrap(), 1);
    }
}
