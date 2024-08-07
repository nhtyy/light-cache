use std::future::Future;
use std::hash::BuildHasher;
use std::hash::Hash;
use std::pin::Pin;
use std::sync::Arc;
use std::task::Context;
use std::task::Poll;

use super::LightCache;
use super::WakerNode;
use crate::map::Shard;

use pin_project::pinned_drop;

#[must_use = "futures do nothing unless you `.await` or poll them"]
#[pin_project::pin_project(project = GetOrInsertFutureProj, PinnedDrop)]
pub enum GetOrInsertFuture<'a, K, V, S, F, Fut>
where
    K: Eq + Hash,
{
    Waiting {
        shard: &'a Shard<K, V>,
        build_hasher: &'a S,
        hash: u64,
        key: K,
        init: Option<F>,
        curr_try: Option<usize>,
    },
    Working {
        shard: &'a Shard<K, V>,
        build_hasher: &'a S,
        hash: u64,
        key: K,
        #[pin]
        fut: Fut,
    },
}

#[pinned_drop]
impl<'a, K, V, S, F, Fut> PinnedDrop for GetOrInsertFuture<'a, K, V, S, F, Fut>
where
    K: Eq + Hash,
{
    fn drop(self: Pin<&mut Self>) {
        let project = self.project();

        match project {
            GetOrInsertFutureProj::Working { shard, key, .. } => {
                let mut waiters = shard.waiters.lock().expect("on drop waiters lock not poisoned");

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

impl<'a, K, V, S, F, Fut> Future for GetOrInsertFuture<'a, K, V, S, F, Fut>
where
    K: Eq + Hash + Copy,
    V: Clone + Sync,
    S: BuildHasher,
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = V>,
{
    type Output = V;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        loop {
            match self.as_mut().project() {
                GetOrInsertFutureProj::Waiting {
                    shard,
                    key,
                    hash,
                    init,
                    curr_try,
                    build_hasher,
                } => {
                    // the value wasnt in the cache, so lets take the lock so no one else is gonna insert it
                    // and we can insert it
                    let mut waiters_lock = shard.waiters.lock().expect("waiters lock not poisoned");
                    if let Some(value) = shard.get(key, *hash) {
                        return Poll::Ready(value);
                    }

                    // lets check if there are any waiters if not we need to compute this ourselves
                    // we dont call `init` just yet as it could panic, we need to drop our locks first.
                    match waiters_lock.get_mut(key) {
                        Some(node) => {
                            if node.is_active() {
                                if let Some(curr_try) = curr_try {
                                    if node.attempts() == curr_try {
                                        return Poll::Pending;
                                    }
                                }

                                // someone is currently trying to insert the value, lets wait
                                node.join(cx.waker().clone());
                                curr_try.replace(*node.attempts());

                                return Poll::Pending;
                            } else {
                                // we have the lock and the last person who had it failed to insert it
                                // so lets signal that we are going to try to insert it
                                curr_try.replace(node.activate());
                            }
                        }
                        None => {
                            // we have the lock and no one is contesting us
                            waiters_lock.insert(*key, WakerNode::start());
                        }
                    };
                    // at this point we havent early returned, so that means this task is going to try to insert the value
                    drop(waiters_lock);

                    let working = GetOrInsertFuture::Working {
                        shard,
                        build_hasher: *build_hasher,
                        hash: *hash,
                        key: *key,
                        fut: init.take().expect("init is none")(),
                    };

                    // saftey: we are moving to the next stage
                    let _ =
                        std::mem::replace(unsafe { self.as_mut().get_unchecked_mut() }, working);
                }
                GetOrInsertFutureProj::Working { shard, key, fut, build_hasher, hash } => {
                    if let Poll::Ready(val) = fut.poll(cx) {
                        let _ = shard.insert(*key, val.clone(), *hash, *build_hasher);

                        let waiters = shard
                            .waiters
                            .lock()
                            .expect("waiters lock not poisoned")
                            .remove(key);

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
        use super::LightCache;

        let cache = LightCache::new();

        let key = 1;
        let val = cache.get_or_insert(key, || async { 1 }).await;

        assert_eq!(val, 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_get_or_insert_many_callers_returns_the_same_value_join() {
        use super::LightCache;

        let cache = LightCache::new();

        let key = 1;

        let fut1 = cache.get_or_insert(key, || async { 1 });
        let fut2 = cache.get_or_insert(key, || async { 2 });
        let fut3 = cache.get_or_insert(key, || async { 3 });

        let (get1, get2, get3) = tokio::join!(fut1, fut2, fut3);

        assert_eq!(get1, 1);
        assert_eq!(get2, 1);
        assert_eq!(get3, 1);
    }
}
