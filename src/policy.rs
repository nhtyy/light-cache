use crate::LightCache;

pub mod noop;
pub mod ttl;

pub use noop::NoopPolicy;
pub use ttl::TtlPolicy;

pub trait Policy<K, V>: Clone {
    fn on_get_or_insert<S>(&self, key: &K, cache: &LightCache<K, V, S, Self>);

    fn on_remove<S>(&self, key: &K, cache: &LightCache<K, V, S, Self>);
}