use std::hash::{BuildHasher, Hash};

use crate::LightCache;

pub mod noop;
pub mod ttl;

pub use noop::NoopPolicy;
pub use ttl::TtlPolicy;

pub trait Policy<K, V>: Clone {
    fn after_get_or_insert<S: BuildHasher>(&self, key: &K, cache: &LightCache<K, V, S, Self>);

    fn after_remove<S: BuildHasher>(&self, key: &K, cache: &LightCache<K, V, S, Self>);
}
