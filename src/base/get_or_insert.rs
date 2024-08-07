use core::fmt;
use std::error::Error;
use std::fmt::Display;
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
    V: Clone + Sync,
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = V>,
{
    type Output = V;

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

                    // lets check if there are any waiters if not we need to compute this ourselves
                    // we dont call `init` just yet as it could panic, we need to drop our locks first.
                    match waiters_lock.get_mut(&key) {
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
                        cache,
                        key: *key,
                        fut: init.take().expect("init is none")(),
                    };

                    // safety: we are moving to the next stage
                    let _ =
                        std::mem::replace(unsafe { self.as_mut().get_unchecked_mut() }, working);
                }
                GetOrInsertFutureProj::Working { cache, key, fut } => {
                    if let Poll::Ready(val) = fut.poll(cx) {
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
                            // safety: we always insert an empty array for the key and we should be the only one to remove it
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

#[must_use = "futures do nothing unless you `.await` or poll them"]
#[pin_project::pin_project(project = GetOrTryInsertFutureProj, PinnedDrop)]
pub enum GetOrTryInsertFuture<'a, K, V, F, Fut>
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
impl<'a, K, V, F, Fut> PinnedDrop for GetOrTryInsertFuture<'a, K, V, F, Fut>
where
    K: Eq + Hash,
{
    fn drop(self: Pin<&mut Self>) {
        let project = self.project();

        match project {
            GetOrTryInsertFutureProj::Working { cache, key, .. } => {
                let mut waiters = cache.waiters.lock().expect("on drop waiters lock not poisoned");

                // notify the wakers and set the node to inactive so another thread can try to insert
                // note: in the happy path the waiters have been removed, so this may be a None
                if let Some(node) = waiters.get_mut(key) {
                    for waker in node.halt() {
                        waker.wake();
                    }
                }
            }
            GetOrTryInsertFutureProj::Waiting { .. } => {}
        }
    }
}

impl<'a, K, V, F, Fut, E> Future for GetOrTryInsertFuture<'a, K, V, F, Fut>
where
    K: Eq + Hash + Copy,
    V: Clone + Sync,
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<V, E>>,
{
    type Output = Result<V, E>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        loop {
            match self.as_mut().project() {
                GetOrTryInsertFutureProj::Waiting {
                    cache,
                    key,
                    init,
                    curr_try,
                } => {
                    // do an initial read to see if the value is in the cache, if it is we can return it
                    // we dont wanna lock the mutex here cause then the reads would be serial
                    if let Some(value) = cache.map.read().expect("rw lock poisioned").get(&key) {
                        return Poll::Ready(Ok(value.clone()));
                    }

                    // the value wasnt in the cache, so lets take the lock so no one else is gonna insert it
                    // and we can insert it
                    let mut waiters_lock = cache.waiters.lock().expect("waiters lock not poisoned");
                    if let Some(value) = cache.map.read().expect("rw lock poisioned").get(&key) {
                        return Poll::Ready(Ok(value.clone()));
                    }

                    // lets check if there are any waiters if not we need to compute this ourselves
                    // we dont call `init` just yet as it could panic, we need to drop our locks first.
                    match waiters_lock.get_mut(&key) {
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

                    let working = GetOrTryInsertFuture::Working {
                        cache,
                        key: *key,
                        fut: init.take().expect("init is none")(),
                    };

                    // safety: we are moving to the next stage
                    let _ =
                        std::mem::replace(unsafe { self.as_mut().get_unchecked_mut() }, working);
                }
                GetOrTryInsertFutureProj::Working { cache, key, fut } => {
                    if let Poll::Ready(val) = fut.poll(cx) {
                        match val {
                            Ok(v) => {
                                cache
                                    .map
                                    .write()
                                    .expect("rw lock poisoned")
                                    .insert(*key, v.clone());

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
                                    // safety: we always insert an empty array for the key and we should be the only one to remove it
                                    unreachable!("no wakers for key, this is a bug");
                                }
                                return Poll::Ready(Ok(v.clone()));
                            }

                            Err(e) => {
                                return Poll::Ready(Err(e));
                            }
                        }
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

    use crate::base::get_or_insert::TestError;

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
        let val: Result<i32, TestError> = cache.get_or_try_insert(key, || async { Ok(1)}).await;

        assert_eq!(val, Ok(1));
    }

    #[tokio::test]
    async fn test_get_or_try_insert_err_single_caller() {
        use super::super::LightCache;

        let cache = LightCache::new();

        let key = 1;
        let val: Result<i32, TestError> = cache.get_or_try_insert(key, || async { Err(TestError::IntentionalError)}).await;

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