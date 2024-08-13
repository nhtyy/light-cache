use crate::cache::DefaultHashBuilder;
use crate::policy::TtlPolicy;
use crate::LightCache;

use std::future::Future;
use std::hash::Hash;
use std::time::Duration;

/// A [`Refresher`] maps a key to a value asynchronously
pub trait Refresher<K, V> {
    type Error;

    fn get(&self, key: K) -> impl Future<Output = Result<V, Self::Error>> + Send;
}

impl<K, V, F, Fut, E> Refresher<K, V> for F
where
    F: Fn(K) -> Fut,
    Fut: Future<Output = Result<V, E>> + Send,
{
    type Error = E;

    fn get(&self, key: K) -> impl Future<Output = Result<V, E>> + Send {
        self(key)
    }
}

/// A [`RefreshCache`] provides a simple interface for caching values with a time-to-live policy
/// for any type that maps a key to a value asynchronously
///
/// This is a replacment for directly using [`LightCache`] 
/// with a [`TtlPolicy`] and calling [`LightCache::get_or_try_insert`] everywhere
pub struct RefreshCache<K, V, R> {
    cache: LightCache<K, V, DefaultHashBuilder, TtlPolicy<K, V>>,
    refresh: R,
}

impl<K, V, R, E> RefreshCache<K, V, R>
where
    K: Copy + Eq + Hash,
    V: Clone + Sync,
    R: Refresher<K, V, Error = E>,
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
