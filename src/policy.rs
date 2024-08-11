use std::future::Future;
use std::hash::{BuildHasher, Hash};

use crate::cache::get_or_insert::GetOrInsertFuture;
use crate::LightCache;

pub mod noop;
pub mod ttl;

mod linked_arena;

pub use noop::NoopPolicy;
pub use ttl::TtlPolicy;

/// A [Policy] tells the [LightCache] what items are allowed to remain in it.
pub trait Policy<K, V>: Clone {
    /// The interal type used by this policy to track keys
    type Node;

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

    fn is_expired(&self, key: &Self::Node) -> bool;
}
