use std::{hash::BuildHasher, sync::{Mutex, MutexGuard}};

use crate::LightCache;
use super::{Policy, Prune};

#[derive(Debug)]
/// A policy that does nothing
/// 
/// While this policy does nothing, we still need to give it a mutex so that it can satisfy the Policy trait
/// It would not be good if a user accidently called prune on a noop policy and got a panic
pub struct NoopPolicy(Mutex<()>);

impl NoopPolicy {
    /// Create a new NoopPolicy
    pub fn new() -> Self {
        NoopPolicy(Mutex::new(()))
    }
}

impl<K, V> Policy<K, V> for NoopPolicy
where
    K: Eq + std::hash::Hash + Copy,
    V: Clone + Sync,
{
    type Inner = ();

    fn lock_inner(&self) -> MutexGuard<()> {
        self.0.lock().unwrap()
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
