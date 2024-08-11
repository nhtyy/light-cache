use std::future::Future;
use std::hash::BuildHasher;
use std::hash::Hash;
use std::pin::Pin;
use std::sync::Arc;
use std::task::Context;
use std::task::Poll;

use super::LightCache;
use crate::map::Shard;
use crate::map::Wakers;

use pin_project::pinned_drop;

#[must_use = "futures do nothing unless you `.await` or poll them"]
#[pin_project::pin_project(project = GetOrInsertFutureProj, PinnedDrop)]
pub enum GetOrInsertFuture<'a, K, V, S, F, Fut>
where
    K: Copy + Eq + Hash,
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
    Ready(Option<V>),
}


#[pinned_drop]
impl<'a, K, V, S, F, Fut> PinnedDrop for GetOrInsertFuture<'a, K, V, S, F, Fut>
where
    K: Copy + Eq + Hash,
{
    fn drop(self: Pin<&mut Self>) {
        let project = self.project();

        match project {
            GetOrInsertFutureProj::Working { shard, key, .. } => {
                let mut waiters = shard
                    .waiters
                    .lock()
                    .expect("on drop waiters lock not poisoned");

                // notify the wakers and set the node to inactive so another thread can try to insert
                // note: in the happy path the waiters have been removed, so this may be a None
                if let Some(node) = waiters.remove(key) {
                    node.alert_all();
                }
            }
            _ => {}
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
                    // take the lock to make sure no one else is trying to insert the value
                    let mut waiters_lock = shard.waiters.lock().expect("waiters lock not poisoned");
                    if let Some(value) = shard.get(key, *hash) {
                        return Poll::Ready(value);
                    }

                    // lets check if there are any waiters if not we need to compute this ourselves
                    // we dont call `init` just yet as it could panic, we need to drop our locks first.
                    match waiters_lock.get_mut(key) {
                        Some(node) => {
                            if let Some(curr_try) = curr_try {
                                if node.attempts() == curr_try {
                                    return Poll::Pending;
                                }
                            }

                            // someone is currently trying to insert the value, lets wait
                            node.join(cx.waker().clone());
                            curr_try.replace(*node.attempts());

                            return Poll::Pending;
                        }
                        None => match curr_try {
                            // we have seen a node before but it has been removed
                            // so we need to increment to tell the next task that the queue has changed
                            Some(curr_try) => {
                                *curr_try += 1;
                                waiters_lock.insert(*key, Wakers::start(*curr_try));
                            }
                            None => {
                                *curr_try = Some(0);
                                waiters_lock.insert(*key, Wakers::start(0));
                            }
                        },
                    };
                    // at this point we havent early returned, so that means this task is going to try to insert the value
                    drop(waiters_lock);

                    // todo avoid these copies somehow ?
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
                GetOrInsertFutureProj::Working {
                    shard,
                    key,
                    fut,
                    build_hasher,
                    hash,
                } => {
                    if let Poll::Ready(val) = fut.poll(cx) {
                        // we have the value, lets insert it into the cache
                        let _ = shard.insert(*key, val.clone(), *hash, *build_hasher);

                        // the waiters will be alerted on drop
                        return Poll::Ready(val);
                    } else {
                        return Poll::Pending;
                    }
                },
                GetOrInsertFutureProj::Ready(v) => {
                    return Poll::Ready(v.take().expect("value is none"));
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
    type Output = Result<V, E>;

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
