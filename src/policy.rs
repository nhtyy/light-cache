use std::hash::BuildHasher;

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

    /// Called before a value is inserted into the cache
    /// Or the key is attempted to be retrieved from the cache
    /// 
    /// In other words: the key may or may not be in cache at this point
    fn before_get_or_insert<S: BuildHasher>(&self, key: &K, cache: &LightCache<K, V, S, Self>);

    /// Called after a successful retrieval of a value from the cache
    /// Or after a value has been inserted into the cache
    /// 
    /// In other words: the key is in the cache at this point
    fn after_get_or_insert<S: BuildHasher>(&self, key: &K, cache: &LightCache<K, V, S, Self>);

    fn after_remove<S: BuildHasher>(&self, key: &K, cache: &LightCache<K, V, S, Self>);

    fn is_expired(&self, key: &Self::Node) -> bool;
}