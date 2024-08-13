use std::{hash::BuildHasher, sync::MutexGuard};

use crate::LightCache;
use super::{Policy, Prune};

#[derive(Clone, Copy, Debug)]
/// A policy that does nothing
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
    fn get<S: BuildHasher>(&self, key: &K, cache: &LightCache<K, V, S, Self>) -> Option<V> {
        cache.get_no_policy(key)
    }

    #[inline]
    fn insert<S: BuildHasher>(&self, key: K, value: V, cache: &LightCache<K, V, S, Self>) -> Option<V> {
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
