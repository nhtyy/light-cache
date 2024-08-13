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
#[pin_project::pin_project(project = Inner, PinnedDrop)]
pub struct GetOrInsertRace<'a, K, V, S, P, Fut>
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
            joined: false,
            shard,
            hash,
            key,
            fut,
        }
    }
}

#[pinned_drop]
impl<'a, K, V, S, P, Fut> PinnedDrop for GetOrInsertRace<'a, K, V, S, P, Fut>
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
        let this = self.as_mut().project();

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
            return Poll::Ready(value);
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
            Poll::Ready(val) => {
                this.cache.insert(*this.key, val.clone());                

                // theres a chance our future resolved immediately after we checked the cache
                // and we didnt even need to insert it
                if let Some(waiters) = lock.remove(this.key) {
                    waiters.finish();
                }

                return Poll::Ready(val);
            }
            Poll::Pending => {
                return Poll::Pending;
            }
        }
    }
}

#[must_use = "futures do nothing unless you `.await` or poll them"]
#[pin_project::pin_project(project = InnerRace, PinnedDrop)]
pub struct GetOrTryInsertRace<'a, K, V, S, P, Fut>
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
            joined: false,
            shard,
            hash,
            key,
            fut,
        }
    }
}

#[pinned_drop]
impl<'a, K, V, S, P, Fut> PinnedDrop for GetOrTryInsertRace<'a, K, V, S, P, Fut>
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
        let this = self.as_mut().project();

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
            return Poll::Ready(Ok(value));
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
            Poll::Ready(Ok(val)) => {
                this.cache.insert(*this.key, val.clone());                

                // theres a chance our future resolved immediately after we checked the cache
                // and we didnt even need to insert it
                if let Some(waiters) = lock.remove(this.key) {
                    waiters.finish();
                }

                return Poll::Ready(Ok(val));
            },
            _ => return maybe_val
        }
    }
}