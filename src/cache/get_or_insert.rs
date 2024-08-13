use std::future::Future;
use std::hash::BuildHasher;
use std::hash::Hash;
use std::pin::Pin;
use std::sync::MutexGuard;
use std::task::ready;
use std::task::Context;
use std::task::Poll;

use crate::map::Shard;
use crate::map::Wakers;
use crate::policy::Policy;

use hashbrown::HashMap;
use pin_project::pinned_drop;

use super::LightCache;

#[pin_project::pin_project(project = GetOrInsertProj)]
pub struct GetOrInsertFuture<'a, K, V, S, P, F, Fut>
where
    K: Copy + Eq + Hash,
{
    cache: &'a LightCache<K, V, S, P>,
    #[pin]
    inner: GetOrInsertFutureInner<'a, K, V, F, Fut>,
}

impl<'a, K, V, S, P, F, Fut> GetOrInsertFuture<'a, K, V, S, P, F, Fut>
where
    K: Copy + Eq + Hash,
{
    pub fn new(
        cache: &'a LightCache<K, V, S, P>,
        shard: &'a Shard<K, V>,
        hash: u64,
        key: K,
        init: F,
    ) -> Self {
        GetOrInsertFuture {
            cache,
            inner: GetOrInsertFutureInner::Waiting {
                shard,
                hash,
                key,
                init: Some(init),
                joined: false,
            },
        }
    }
}

impl<'a, K, V, S, P, F, Fut> Future for GetOrInsertFuture<'a, K, V, S, P, F, Fut>
where
    K: Eq + Hash + Copy,
    V: Clone + Sync,
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = V>,
    P: Policy<K, V>,
    S: BuildHasher,
{
    type Output = V;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {        
        match ready!(self.as_mut().project().inner.poll(cx)) {
            InnerComplete::Insert(key, value, mut lock) => {
                self.cache.insert(key, value.clone());

                if let Some(wakers) = lock.remove(&key) {
                    wakers.finish();
                } else {
                    unreachable!("Were holding the lock and were told to insert yet we dont have a waiter, this is a bug");
                }

                Poll::Ready(value)
            },
            InnerComplete::Got(value) => {
                Poll::Ready(value)
            },
        }
    }
}

#[pin_project::pin_project(project = GetOrTryInsertProj)]
pub struct GetOrTryInsertFuture<'a, K, V, S, P, F, Fut>
where
    K: Copy + Eq + Hash,
{
    cache: &'a LightCache<K, V, S, P>,
    #[pin]
    inner: GetOrInsertFutureInner<'a, K, V, F, Fut>,
}

impl<'a, K, V, S, P, F, Fut> GetOrTryInsertFuture<'a, K, V, S, P, F, Fut>
where
    K: Copy + Eq + Hash,
{
    pub fn new(
        cache: &'a LightCache<K, V, S, P>,
        shard: &'a Shard<K, V>,
        hash: u64,
        key: K,
        init: F,
    ) -> Self {
        GetOrTryInsertFuture {
            cache,
            inner: GetOrInsertFutureInner::Waiting {
                shard,
                hash,
                key,
                init: Some(init),
                joined: false,
            },
        }
    }
}

impl<'a, K, V, S, P, F, Fut, Err> Future for GetOrTryInsertFuture<'a, K, V, S, P, F, Fut>
where
    K: Eq + Hash + Copy,
    V: Clone + Sync,
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<V, Err>>,
    P: Policy<K, V>,
    S: BuildHasher,
{
    type Output = Result<V, Err>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match ready!(self.as_mut().project().inner.poll(cx)) {
            InnerComplete::Insert(key, value, mut lock) => {
                match value {
                    Ok(value) => {
                        self.cache.insert(key, value.clone());

                        if let Some(wakers) = lock.remove(&key) {
                            wakers.finish();
                        } else {
                            unreachable!("Were holding the lock and were told to insert yet we dont have a waiter, this is a bug");
                        }

                        Poll::Ready(Ok(value))
                    },
                    Err(err) => {
                        // drop impl will decrement the worker count
                        Poll::Ready(Err(err))
                    }
                }
            },
            InnerComplete::Got(value) => {
                Poll::Ready(Ok(value))
            },
        }
    }
}

#[must_use = "futures do nothing unless you `.await` or poll them"]
#[pin_project::pin_project(project = GetOrTryInsertFutureInnerProj, PinnedDrop)]
pub enum GetOrInsertFutureInner<'a, K, V, F, Fut>
where
    K: Copy + Eq + Hash,
{
    Waiting {
        shard: &'a Shard<K, V>,
        hash: u64,
        key: K,
        init: Option<F>,
        joined: bool,
    },
    Working {
        shard: &'a Shard<K, V>,
        hash: u64,
        key: K,
        #[pin]
        fut: Fut,
    },
}

#[pinned_drop]
impl<'a, K, V, F, Fut> PinnedDrop for GetOrInsertFutureInner<'a, K, V, F, Fut>
where
    K: Copy + Eq + Hash,
{
    fn drop(self: Pin<&mut Self>) {
        let project = self.project();

        match project {
            GetOrTryInsertFutureInnerProj::Working { shard, key, .. } => {
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

pub enum InnerComplete<'a, K, V, O> {
    Insert(K, O, MutexGuard<'a, HashMap<K, Wakers>>),
    Got(V),
}

impl<'a, K, V, F, O, Fut> Future for GetOrInsertFutureInner<'a, K, V, F, Fut>
where
    K: Eq + Hash + Copy,
    V: Clone + Sync,
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = O>,
{
    type Output = InnerComplete<'a, K, V, O>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<InnerComplete<'a, K, V, O>> {
        loop {
            match self.as_mut().project() {
                GetOrTryInsertFutureInnerProj::Waiting {
                    shard,
                    hash,
                    key,
                    init,
                    joined,
                } => {
                    // take the lock to make sure no one else is trying to insert the value
                    let mut waiters_lock = shard.waiters.lock().expect("waiters lock not poisoned");
                    if let Some(value) = shard.get(key, *hash) {
                        return Poll::Ready(InnerComplete::Got(value));
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
                                // we are the only worker, lets join as a worker
                                node.join_worker();
                            }
                        }
                        None => {
                            waiters_lock.insert(*key, Wakers::start(cx.waker().clone()));
                        }
                    };
                    // at this point we havent early returned, so that means this task is going to try to insert the value
                    // drop the lock in case init call panics
                    drop(waiters_lock);

                    // todo avoid these copies somehow ?
                    let working = GetOrInsertFutureInner::Working {
                        shard,
                        hash: *hash,
                        key: *key,
                        fut: init.take().expect("init is none")(),
                    };

                    // saftey: we are moving to the next stage
                    let _ =
                        std::mem::replace(unsafe { self.as_mut().get_unchecked_mut() }, working);
                }
                GetOrTryInsertFutureInnerProj::Working {
                    shard,
                    hash,
                    key,
                    fut,
                } => match fut.poll(cx) {
                    Poll::Ready(out) => {
                        let lock = shard.waiters.lock().expect("waiters lock not poisoned");
                        if let Some(value) = shard.get(key, *hash) {
                            return Poll::Ready(InnerComplete::Got(value));
                        }

                        // todo remove this copy?
                        return Poll::Ready(InnerComplete::Insert(*key, out, lock));
                    }
                    Poll::Pending => return Poll::Pending,
                },
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
