use core::fmt;
use std::error::Error;
use std::fmt::Display;
use std::future::Future;
use std::hash::BuildHasher;
use std::hash::Hash;
use std::pin::Pin;
use std::sync::Arc;
use std::task::Context;
use std::task::Poll;

use super::LightCache;
use crate::map::Shard;
use crate::waker_node::Wakers;

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
                }
            }
        }
    }
}


#[must_use = "futures do nothing unless you `.await` or poll them"]
#[pin_project::pin_project(project = GetOrTryInsertFutureProj, PinnedDrop)]
pub enum GetOrTryInsertFuture<'a, K, V, S, F, Fut, E>
where
    K: Copy + Eq + Hash,
{
    Waiting {
        shard: &'a Shard<K, Result<V, E>>,
        build_hasher: &'a S,
        hash: u64,
        key: K,
        init: Option<F>,
        curr_try: Option<usize>,
    },
    Working {
        shard: &'a Shard<K, Result<V, E>>,
        build_hasher: &'a S,
        hash: u64,
        key: K,
        #[pin]
        fut: Fut,
    },
}

#[pinned_drop]
impl<'a, K, V, F, S, Fut, E> PinnedDrop for GetOrTryInsertFuture<'a, K, V, F, S, Fut, E>
where
    K: Copy + Eq + Hash,
{
    fn drop(self: Pin<&mut Self>) {
        let project = self.project();

        match project {
            GetOrTryInsertFutureProj::Working { shard, key, .. } => {
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
            GetOrTryInsertFutureProj::Waiting { .. } => {}
        }
    }
}

impl<'a, K, V, F, S, Fut, E> Future for GetOrTryInsertFuture<'a, K, V, F, S, Fut, E>
where
    K: Eq + Hash + Copy,
    E: Sized + Clone,
    V: Clone + Sync,
    S: BuildHasher,
    F: FnOnce() -> Fut + BuildHasher,
    Fut: std::future::Future<Output = V>,
{
    type Output = Result<V, E>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        loop {
            match self.as_mut().project() {
                GetOrTryInsertFutureProj::Waiting {
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
                        match value {
                            Ok(val) => {
                                return Poll::Ready(Ok(val));
                            }
                            Err(e) => {
                                return Poll::Ready(Err(e));
                            }
                        }
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
                GetOrTryInsertFutureProj::Working {
                    shard,
                    key,
                    fut,
                    build_hasher,
                    hash,
                } => {
                    if let Poll::Ready(val) = fut.poll(cx) {
                        // we have the value, lets insert it into the cache
                        let _ = shard.insert(*key, Ok(val.clone()), *hash, *build_hasher);
                        // the waiters will be alerted on drop
                        return Poll::Ready(Ok(val));
                    } else {
                        return Poll::Pending;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod test {
    use tokio::task::JoinHandle;

    use crate::cache::get_or_insert::TestError;

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

        assert_eq!(get1.unwrap(), 1);
        assert_eq!(get2.unwrap(), 1);
        assert_eq!(get3.unwrap(), 1);
    }

    #[tokio::test]
    async fn test_get_or_try_insert_ok_single_caller() {
        use super::super::LightCache;

        let cache = LightCache::new();

        let key = 1;
        let val: Result<i32, TestError> = cache.get_or_try_insert(key, || async { Ok(1)});

        assert_eq!(val, Ok(1));
    }

    #[tokio::test]
    async fn test_get_or_try_insert_err_single_caller() {
        use super::super::LightCache;

        let cache = LightCache::new();

        let key = 1;
        let val: Result<i32, TestError> = cache.get_or_try_insert(key, || async { Err(TestError::IntentionalError)});

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
        let fut3 = cache.get_or_try_insert(key, || async { Ok::<i32, TestError>(3)});

        let (get1, get2, get3) = tokio::join!(fut1, fut2, fut3);

        assert_eq!(get1, Err(TestError::IntentionalError));
        assert_eq!(get2, Ok(2));
        assert_eq!(get3, Ok(2));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_get_or_try_insert_ok_many_callers_returns_the_same_value_spawn() {
        use super::super::LightCache;

        let cache = LightCache::new();

        let key = 1;

        let get1: JoinHandle<Result<i32, TestError>> = tokio::spawn({
            let cache = cache.clone();
            async move {
                // no sleep
                cache.get_or_try_insert(key, || async { Ok(1) }).await
            }
        });
        let get2: JoinHandle<Result<i32, TestError>> = tokio::spawn({
            let cache = cache.clone();
            async move {
                cache
                    .get_or_try_insert(key, || async {
                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                        Ok(2)
                    })
                    .await
            }
        });
        let get3: JoinHandle<Result<i32, TestError>> = tokio::spawn({
            let cache = cache.clone();
            async move {
                cache
                    .get_or_try_insert(key, || async {
                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                        Ok(3)
                    })
                    .await
            }
        });

        let (get1, get2, get3) = tokio::join!(get1, get2, get3);

        assert_eq!(get1.unwrap(), Ok(1));
        assert_eq!(get2.unwrap(), Ok(1));
        assert_eq!(get3.unwrap(), Ok(1));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_get_or_try_insert_err_many_callers_returns_the_same_value_spawn() {
        use super::super::LightCache;

        let cache = LightCache::new();

        let key = 1;

        let get1 = tokio::spawn({
            let cache = cache.clone();
            async move {
                // no sleep
                cache.get_or_try_insert(key, || async { Err(TestError::IntentionalError) }).await
            }
        });
        let get2: JoinHandle<Result<i32, TestError>> = tokio::spawn({
            let cache = cache.clone();
            async move {
                cache
                    .get_or_try_insert(key, || async {
                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                        Ok(2)
                    })
                    .await
            }
        });
        let get3: JoinHandle<Result<i32, TestError>> = tokio::spawn({
            let cache = cache.clone();
            async move {
                cache
                    .get_or_try_insert(key, || async {
                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                        Ok(3)
                    })
                    .await
            }
        });

        let (get1, get2, get3) = tokio::join!(get1, get2, get3);

        assert_eq!(get1.unwrap(), Err(TestError::IntentionalError));
        assert_eq!(get2.unwrap(), Ok(2));
        assert_eq!(get3.unwrap(), Ok(2));
    }
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum TestError {
    IntentionalError
}

impl Error for TestError {}
impl Display for TestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TestError::IntentionalError => write!(f, ""),
        }
    }
}