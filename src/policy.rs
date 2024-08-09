use std::hash::BuildHasher;

use crate::LightCache;

pub mod noop;
pub mod ttl;

mod linked_arena;

pub use noop::NoopPolicy;
pub use ttl::TtlPolicy;

pub trait Policy<K, V>: Clone {
    type Node;
    type Expiry: Expiry<Self::Node>;

    fn after_get_or_insert<S: BuildHasher>(&self, key: &K, cache: &LightCache<K, V, S, Self>);

    fn after_remove<S: BuildHasher>(&self, key: &K, cache: &LightCache<K, V, S, Self>);
}

pub trait Expiry<N> {
    fn is_expired(&self, key: &N) -> bool;
}
