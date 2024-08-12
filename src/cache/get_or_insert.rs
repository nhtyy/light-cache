use std::future::Future;
use std::hash::BuildHasher;
use std::hash::Hash;
use std::pin::Pin;
use std::task::Context;
use std::task::Poll;

use crate::map::Shard;
use crate::map::Wakers;
use crate::policy::Policy;

use pin_project::pinned_drop;

use super::LightCache;

/// Todo can we make this more generic? theres alot of repetition here

#[must_use = "futures do nothing unless you `.await` or poll them"]
#[pin_project::pin_project(project = GetOrInsertFutureProj, PinnedDrop)]
pub enum GetOrInsertFuture<'a, K, V, S, P, F, Fut>
where
    K: Copy + Eq + Hash,
{
    Waiting {
        shard: &'a Shard<K, V>,
        cache: &'a LightCache<K, V, S, P>,
        hash: u64,
        key: K,
        init: Option<F>,
        joined: bool,
    },
    Working {
        shard: &'a Shard<K, V>,
        cache: &'a LightCache<K, V, S, P>,
        hash: u64,
        key: K,
        #[pin]
        fut: Fut,
    },
    Ready(Option<V>),
}

#[pinned_drop]
impl<'a, K, V, S, P, F, Fut> PinnedDrop for GetOrInsertFuture<'a, K, V, S, P, F, Fut>
where
    K: Copy + Eq + Hash,
{
    fn drop(self: Pin<&mut Self>) {
        let project = self.project();

        match project {
            GetOrInsertFutureProj::Working { shard, key, .. } => {
                let mut lock = shard.waiters.lock().expect("waiters lock not poisoned");

                if let Some(node) = lock.get_mut(key) {
                    // if we hit this branch than we panicked and we didnt finish insertion
                    node.remove_worker();
                    node.alert_all();
                }
            }
            _ => {}
        }
    }
}

impl<'a, K, V, S, P, F, Fut> Future for GetOrInsertFuture<'a, K, V, S, P, F, Fut>
where
    K: Eq + Hash + Copy,
    V: Clone + Sync,
    S: BuildHasher,
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = V>,
    P: Policy<K, V>,
{
    type Output = V;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        loop {
            match self.as_mut().project() {
                GetOrInsertFutureProj::Waiting {
                    shard,
                    key,
                    init,
                    joined,
                    cache,
                    hash,
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
                            if !*joined {
                                node.join_waiter(cx.waker().clone());
                                *joined = true;
                            }

                            if node.workers > 0 {
                                return Poll::Pending;
                            } else {
                                // we are the only worker, lets join the worker
                                node.join_worker();
                            }
                        }
                        None => {
                            waiters_lock.insert(*key, Wakers::start(cx.waker().clone()));
                        }
                    };
                    // at this point we havent early returned, so that means this task is going to try to insert the value
                    drop(waiters_lock);

                    // todo avoid these copies somehow ?
                    let working = GetOrInsertFuture::Working {
                        shard,
                        cache: *cache,
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
                    hash,
                    cache,
                } => {
                    if let Poll::Ready(val) = fut.poll(cx) {
                        let mut lock = shard.waiters.lock().expect("waiters lock not poisoned");
                        if let Some(value) = shard.get(key, *hash) {
                            return Poll::Ready(value);
                        }

                        cache.insert(*key, val.clone());

                        if let Some(wakers) = lock.remove(key) {
                            wakers.finsih();
                        } else {
                            unreachable!("were holding the lock and inserted the value into the cache, this is a bug");
                        }

                        // the waiters will be alerted on drop
                        return Poll::Ready(val);
                    } else {
                        return Poll::Pending;
                    }
                }
                GetOrInsertFutureProj::Ready(v) => {
                    return Poll::Ready(v.take().expect("value is none"));
                }
            }
        }
    }
}

#[must_use = "futures do nothing unless you `.await` or poll them"]
#[pin_project::pin_project(project = GetOrTryInsertFutureProj, PinnedDrop)]
pub enum GetOrTryInsertFuture<'a, K, V, S, P, F, Fut>
where
    K: Copy + Eq + Hash,
{
    Waiting {
        cache: &'a LightCache<K, V, S, P>,
        shard: &'a Shard<K, V>,
        hash: u64,
        key: K,
        init: Option<F>,
        joined: bool,
    },
    Working {
        cache: &'a LightCache<K, V, S, P>,
        shard: &'a Shard<K, V>,
        hash: u64,
        key: K,
        #[pin]
        fut: Fut,
    },
    Ready(Option<V>),
}

#[pinned_drop]
impl<'a, K, V, S, P, F, Fut> PinnedDrop for GetOrTryInsertFuture<'a, K, V, S, P, F, Fut>
where
    K: Copy + Eq + Hash,
{
    fn drop(self: Pin<&mut Self>) {
        let project = self.project();

        match project {
            GetOrTryInsertFutureProj::Working { shard, key, .. } => {
                let mut lock = shard.waiters.lock().expect("waiters lock not poisoned");

                if let Some(node) = lock.get_mut(key) {
                    // if we hit this branch than we retunred an error, and we didnt finish insertion
                    node.remove_worker();
                    node.alert_all();
                }
            }
            _ => {}
        }
    }
}

impl<'a, K, V, S, P, F, Fut, E> Future for GetOrTryInsertFuture<'a, K, V, S, P, F, Fut>
where
    K: Eq + Hash + Copy,
    V: Clone + Sync,
    S: BuildHasher,
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<V, E>>,
    P: Policy<K, V>,
{
    type Output = Result<V, E>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        loop {
            match self.as_mut().project() {
                GetOrTryInsertFutureProj::Waiting {
                    shard,
                    key,
                    init,
                    joined,
                    cache,
                    hash,
                } => {
                    // take the lock to make sure no one else is trying to insert the value
                    let mut waiters_lock = shard.waiters.lock().expect("waiters lock not poisoned");
                    if let Some(value) = shard.get(key, *hash) {
                        return Poll::Ready(Ok(value));
                    }

                    // lets check if there are any waiters if not we need to compute this ourselves
                    // we dont call `init` just yet as it could panic, we need to drop our locks first.
                    match waiters_lock.get_mut(key) {
                        Some(node) => {
                            if !*joined {
                                node.join_waiter(cx.waker().clone());
                                *joined = true;
                            }

                            if node.workers > 0 {
                                return Poll::Pending;
                            } else {
                                // we are the only worker, lets join the worker
                                node.join_worker();
                            }
                        }
                        None => {
                            waiters_lock.insert(*key, Wakers::start(cx.waker().clone()));
                        }
                    };
                    // at this point we havent early returned, so that means this task is going to try to insert the value
                    drop(waiters_lock);

                    // todo avoid these copies somehow ?
                    let working = GetOrTryInsertFuture::Working {
                        shard,
                        cache: *cache,
                        hash: *hash,
                        key: *key,
                        fut: init.take().expect("init is none")(),
                    };

                    // saftey: we are moving to the next stage
                    let _ =
                        std::mem::replace(unsafe { self.as_mut().get_unchecked_mut() }, working);
                }
                GetOrTryInsertFutureProj::Working {
                    cache,
                    shard,
                    key,
                    fut,
                    hash,
                } => {
                    match fut.poll(cx) {
                        Poll::Ready(Ok(val)) => {
                            let mut lock = shard.waiters.lock().expect("waiters lock not poisoned");
                            if let Some(value) = shard.get(key, *hash) {
                                return Poll::Ready(Ok(value));
                            }

                            cache.insert(*key, val.clone());

                            if let Some(wakers) = lock.remove(key) {
                                wakers.finsih();
                            } else {
                                unreachable!("were holding the lock and inserted the value into the cache, this is a bug");
                            }

                            return Poll::Ready(Ok(val));
                        }
                        Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                        Poll::Pending => return Poll::Pending,
                    }
                }
                GetOrTryInsertFutureProj::Ready(v) => {
                    return Poll::Ready(Ok(v.take().expect("Value is none in the future")));
                }
            }
        }
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

        assert_eq!(val, 1);
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

        assert_eq!(get1, 1);
        assert_eq!(get2, 1);
        assert_eq!(get3, 1);
    }

    #[tokio::test]
    async fn test_get_or_try_insert_ok_single_caller() {
        use super::super::LightCache;

        let cache = LightCache::new();

        let key = 1;
        let val: Result<i32, TestError> = cache.get_or_try_insert(key, || async { Ok(1) }).await;

        assert_eq!(val, Ok(1));
    }

    #[tokio::test]
    async fn test_get_or_try_insert_err_single_caller() {
        use super::super::LightCache;

        let cache = LightCache::new();

        let key = 1;
        let val: Result<i32, TestError> = cache
            .get_or_try_insert(key, || async { Err(TestError::IntentionalError) })
            .await;

        assert_eq!(val, Err(TestError::IntentionalError));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_get_or_try_insert_ok_many_callers_returns_the_same_value_join() {
        use super::super::LightCache;

        let cache = LightCache::new();

        let key = 1;

        let fut1 = cache.get_or_try_insert(key, || async { Ok::<i32, TestError>(1) });
        let fut2 = cache.get_or_try_insert(key, || async { Ok::<i32, TestError>(2) });
        let fut3 = cache.get_or_try_insert(key, || async { Ok::<i32, TestError>(3) });

        let (get1, get2, get3) = tokio::join!(fut1, fut2, fut3);

        assert_eq!(get1, Ok(1));
        assert_eq!(get2, Ok(1));
        assert_eq!(get3, Ok(1));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_get_or_try_insert_err_many_callers_returns_the_same_value_join() {
        use super::super::LightCache;

        let cache: LightCache<i32, i32> = LightCache::new();

        let key = 1;

        let fut1 = cache.get_or_try_insert(key, || async { Err(TestError::IntentionalError) });
        let fut2 = cache.get_or_try_insert(key, || async { Ok::<i32, TestError>(2) });
        let fut3 = cache.get_or_try_insert(key, || async { Ok::<i32, TestError>(3) });

        let (get1, get2, get3) = tokio::join!(fut1, fut2, fut3);

        assert_eq!(get1, Err(TestError::IntentionalError));
        assert_eq!(get2, Ok(2));
        assert_eq!(get3, Ok(2));
    }

    #[derive(Debug, PartialEq, Clone, Copy)]
    pub enum TestError {
        IntentionalError,
    }

    impl std::error::Error for TestError {}
    impl std::fmt::Display for TestError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                TestError::IntentionalError => write!(f, ""),
            }
        }
    }
}
