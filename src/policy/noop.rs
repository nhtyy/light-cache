use std::{hash::BuildHasher, sync::MutexGuard};

use crate::{cache::get_or_insert::GetOrInsertFuture, LightCache};

use super::{Policy, Prune};

#[derive(Clone, Copy, Debug)]
pub struct NoopPolicy;

impl<K, V> Policy<K, V> for NoopPolicy
where
    K: Eq + std::hash::Hash + Copy,
    V: Clone + Sync,
{
    type Inner = ();

    fn lock_inner(&self) -> MutexGuard<()> {
        unreachable!("You should not be calling inner on noop policy")
    }

    #[inline]
    fn get_or_insert<'a, S, F, Fut>(
        &self,
        key: K,
        cache: &'a LightCache<K, V, S, Self>,
        init: F,
    ) -> GetOrInsertFuture<'a, K, V, S, F, Fut>
    where
        S: BuildHasher,
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = V>,
    {
        cache.get_or_insert_no_policy(key, init)
    }

    #[inline]
    fn get<S: BuildHasher>(&self, key: &K, cache: &LightCache<K, V, S, Self>) -> Option<V> {
        cache.get_no_policy(key)
    }

    #[inline]
    fn insert<S: BuildHasher>(&self, key: K, value: V, cache: &LightCache<K, V, S, Self>) {
        cache.insert_no_policy(key, value)
    }

    #[inline]
    fn remove<S: BuildHasher>(&self, key: &K, cache: &LightCache<K, V, S, Self>) -> Option<V> {
        cache.remove_no_policy(key)
    }
}

impl<K, V> Prune<K, V, NoopPolicy> for () {
    fn prune<S: BuildHasher>(&mut self, _: &LightCache<K, V, S, NoopPolicy>) {}
}
