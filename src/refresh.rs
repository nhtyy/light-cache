use crate::cache::DefaultHashBuilder;
use crate::policy::TtlPolicy;
use crate::LightCache;

use std::future::Future;
use std::hash::Hash;
use std::time::Duration;

/// A `Refresh` is a type with a method [Refresh::get] that maps a key to a value asynchronously
pub trait Refresh<K, V> {
    type Error;

    fn get(&self, key: K) -> impl Future<Output = Result<V, Self::Error>>;
}

impl<K, V, F, Fut, E> Refresh<K, V> for F
where
    F: Fn(K) -> Fut,
    Fut: Future<Output = Result<V, E>>,
{
    type Error = E;

    fn get(&self, key: K) -> impl Future<Output = Result<V, E>> {
        self(key)
    }
}

/// The [`RefreshCache`] is a cache that will refresh the value if it is expired
/// It must be created with a [`Refresh`] implementation that will fetch the value from an external source
/// 
/// This is a replacment for directly using [`LightCache`] 
/// with a [`TtlPolicy`] and calling [`LightCache::get_or_try_insert`] everywhere
pub struct RefreshCache<K, V, R> {
    pub cache: LightCache<K, V, DefaultHashBuilder, TtlPolicy<K, V>>,
    refresh: R,
}

impl<K, V, R, E> RefreshCache<K, V, R>
where
    K: Copy + Eq + Hash,
    V: Clone + Sync,
    R: Refresh<K, V, Error = E>,
{
    pub fn new(refresh: R, ttl: Duration) -> Self {
        Self {
            cache: LightCache::from_parts(TtlPolicy::new(ttl), Default::default()),
            refresh,
        }
    }

    pub async fn get(&self, key: K) -> Result<V, R::Error> {
        self.cache.get_or_try_insert(key, || self.refresh.get(key)).await
    }
}
