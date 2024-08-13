use std::future::Future;
use std::hash::BuildHasher;
use std::hash::Hash;
use std::pin::Pin;
use std::task::ready;
use std::task::Context;
use std::task::Poll;

use crate::map::Shard;
use crate::map::Wakers;
use crate::policy::Policy;

use pin_project::pinned_drop;

use super::get_or_insert::InnerComplete;
use super::LightCache;

#[must_use = "futures do nothing unless you `.await` or poll them"]
#[pin_project::pin_project(project = GetOrInsertRaceProj)]
pub struct GetOrInsertRace<'a, K, V, S, P, Fut>
where
    K: Copy + Eq + Hash,
{
    cache: &'a LightCache<K, V, S, P>,
    #[pin]
    inner: RaceInner<'a, K, V, S, P, Fut>,
}

impl<'a, K, V, S, P, Fut> GetOrInsertRace<'a, K, V, S, P, Fut>
where
    K: Copy + Eq + Hash,
{
    pub fn new(
        cache: &'a LightCache<K, V, S, P>,
        shard: &'a Shard<K, V>,
        hash: u64,
        key: K,
        fut: Fut,
    ) -> Self {
        GetOrInsertRace {
            cache,
            inner: RaceInner {
                cache,
                shard,
                joined: false,
                hash,
                key,
                fut,
            },
        }
    }
}

impl<'a, K, V, S, P, Fut> Future for GetOrInsertRace<'a, K, V, S, P, Fut>
where
    K: Eq + Hash + Copy,
    V: Clone + Sync,
    S: BuildHasher,
    Fut: std::future::Future<Output = V>,
    P: Policy<K, V>,
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

#[must_use = "futures do nothing unless you `.await` or poll them"]
#[pin_project::pin_project(project = GetOrTryInsertRaceProj)]
pub struct GetOrTryInsertRace<'a, K, V, S, P, Fut>
where
    K: Copy + Eq + Hash,
{
    cache: &'a LightCache<K, V, S, P>,
    #[pin]
    inner: RaceInner<'a, K, V, S, P, Fut>,
}

impl<'a, K, V, S, P, Fut> GetOrTryInsertRace<'a, K, V, S, P, Fut>
where
    K: Copy + Eq + Hash,
{
    pub fn new(
        cache: &'a LightCache<K, V, S, P>,
        shard: &'a Shard<K, V>,
        hash: u64,
        key: K,
        fut: Fut,
    ) -> Self {
        GetOrTryInsertRace {
            cache,
            inner: RaceInner {
                cache,
                shard,
                joined: false,
                hash,
                key,
                fut,
            },
        }
    }
}

impl<'a, K, V, S, P, Fut, Err> Future for GetOrTryInsertRace<'a, K, V, S, P, Fut>
where
    K: Eq + Hash + Copy,
    V: Clone + Sync,
    S: BuildHasher,
    Fut: std::future::Future<Output = Result<V, Err>>,
    P: Policy<K, V>,
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
                    }
                    Err(err) => {
                        // drop impl will decrement the worker count
                        Poll::Ready(Err(err))
                    }
                }
            }
            InnerComplete::Got(value) => Poll::Ready(Ok(value)),
        }
    }
}

#[must_use = "futures do nothing unless you `.await` or poll them"]
#[pin_project::pin_project(project = Inner, PinnedDrop)]
struct RaceInner<'a, K, V, S, P, Fut>
where
    K: Copy + Eq + Hash,
{
    cache: &'a LightCache<K, V, S, P>,
    shard: &'a Shard<K, V>,
    joined: bool,
    hash: u64,
    key: K,
    #[pin]
    fut: Fut,
}

#[pinned_drop]
impl<'a, K, V, S, P, Fut> PinnedDrop for RaceInner<'a, K, V, S, P, Fut>
where
    K: Copy + Eq + Hash,
{
    fn drop(self: Pin<&mut Self>) {
        let this = self.project();

        // if were dropping on an occupied entry we panicked
        if *this.joined {
            let mut lock = this
                .shard
                .waiters
                .lock()
                .expect("waiters lock not poisoned");

            if let Some(node) = lock.get_mut(this.key) {
                node.remove_worker();
                node.alert_all();
            }
        }
    }
}

impl<'a, K, V, S, P, Fut> Future for RaceInner<'a, K, V, S, P, Fut>
where
    K: Eq + Hash + Copy,
    V: Clone + Sync,
    S: BuildHasher,
    Fut: std::future::Future,
    P: Policy<K, V>,
{
    type Output = InnerComplete<'a, K, V, Fut::Output>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();

        // Poll this tasks future first to get it out of the way before we take the lock
        let maybe_val = this.fut.poll(cx);

        // hold the lock so no one can insert it
        let mut lock = this
            .shard
            .waiters
            .lock()
            .expect("waiters lock not poisoned");

        // lets early return if its already in the cache
        if let Some(value) = this.shard.get(&this.key, *this.hash) {
            return Poll::Ready(InnerComplete::Got(value));
        }

        // if our value isnt ready lets make sure we get woken up or alert others when we insert it
        if maybe_val.is_pending() {
            match lock.get_mut(this.key) {
                Some(node) => {
                    if !*this.joined {
                        // someone else is also currently trying to insert the value, lets make sure were woken up when they insert too
                        node.join_waiter(cx.waker().clone());
                        node.join_worker();

                        *this.joined = true;
                    }
                }
                None => {
                    lock.insert(*this.key, Wakers::start(cx.waker().clone()));
                }
            }
        }

        match maybe_val {
            Poll::Ready(out) => {
                return Poll::Ready(InnerComplete::Insert(*this.key, out, lock));
            }
            Poll::Pending => {
                return Poll::Pending;
            }
        }
    }
}
