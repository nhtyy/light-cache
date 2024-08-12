use std::future::Future;
use std::hash::{BuildHasher, Hash};
use std::sync::MutexGuard;

use crate::cache::get_or_insert::GetOrInsertFuture;
use crate::LightCache;

pub mod noop;
pub mod ttl;

mod linked_arena;

pub use noop::NoopPolicy;
pub use ttl::TtlPolicy;

/// A [Policy] augments a [LightCache] instance, managing the entry and eviction of items in the cache
/// 
/// A policy usally requires shared mutable state, therefore the [`Policy::Inner`] type is used to represent this.
pub trait Policy<K, V>: Clone {
    /// The inner type of this policy, likely behind a lock
    type Inner: Prune<K, V, Self>;

    fn lock_inner(&self) -> MutexGuard<'_, Self::Inner>;

    fn get_or_insert<'a, S, F, Fut>(
        &self,
        key: K,
        cache: &'a LightCache<K, V, S, Self>,
        init: F,
    ) -> GetOrInsertFuture<'a, K, V, S, F, Fut>
    where
        K: Eq + Hash + Copy,
        S: BuildHasher,
        F: FnOnce() -> Fut,
        Fut: Future<Output = V>;

    fn get<S: BuildHasher>(&self, key: &K, cache: &LightCache<K, V, S, Self>) -> Option<V>;

    fn insert<S: BuildHasher>(&self, key: K, value: V, cache: &LightCache<K, V, S, Self>);

    fn remove<S: BuildHasher>(&self, key: &K, cache: &LightCache<K, V, S, Self>) -> Option<V>;

    /// # Warning: Calling this while holding a lock from [`Policy::lock_inner`] will deadlock
    fn prune<S: BuildHasher>(&self, cache: &LightCache<K, V, S, Self>) {
        self.lock_inner().prune(cache)
    }
}

/// [Prune] controls how entries are expired (not nescessarily evicted) from the cache
pub trait Prune<K, V, P> {
    /// Prune is typically be called before any operation on the cahce, or the policy
    /// 
    /// For instance: An LRU w/ expiration would prune expired entries before checking if its full
    fn prune<S: BuildHasher>(&mut self, cache: &LightCache<K, V, S, P>);
}
