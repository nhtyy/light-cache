use std::hash::BuildHasher;
use std::sync::MutexGuard;

use crate::LightCache;

pub mod lru;
pub mod noop;
pub mod ttl;

mod linked_arena;

pub use noop::NoopPolicy;
pub use ttl::TtlPolicy;

/// A [Policy] augments accsess to a [LightCache] instance, managing the entry and eviction of items in the cache.
///
/// A policy usally requires shared mutable state, therefore the [`Policy::Inner`] type is used to represent this.
pub trait Policy<K, V>: Sized {
    /// The inner type of this policy, likely behind a lock
    type Inner: Prune<K, V, Self>;

    /// # Panics
    /// This method will panic if the lock is poisoned
    fn lock_inner(&self) -> MutexGuard<'_, Self::Inner>;
    
    fn get<S: BuildHasher>(&self, key: &K, cache: &LightCache<K, V, S, Self>) -> Option<V>;

    fn insert<S: BuildHasher>(&self, key: K, value: V, cache: &LightCache<K, V, S, Self>) -> Option<V>;

    fn remove<S: BuildHasher>(&self, key: &K, cache: &LightCache<K, V, S, Self>) -> Option<V>;

    /// # Warning: 
    /// Calling this method while holding a lock from [`Policy::lock_inner`] will cause a deadlock
    fn prune<S: BuildHasher>(&self, cache: &LightCache<K, V, S, Self>) {
        self.lock_inner().prune(cache)
    }
}

/// [Prune] should control how entries are expired (not nescessarily evicted) from the cache
pub trait Prune<K, V, P> {
    /// Prune is typically be called before any operation on the cahce
    ///
    /// For instance: An LRU w/ expiration would prune expired entries before checking if its full
    fn prune<S: BuildHasher>(&mut self, cache: &LightCache<K, V, S, P>);
}
