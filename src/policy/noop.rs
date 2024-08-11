use super::Policy;

#[derive(Clone, Copy, Debug)]
pub struct NoopPolicy;
pub struct NoopExpiry;

impl<K, V> Policy<K, V> for NoopPolicy {
    type Node = K;

    #[inline]
    fn before_get_or_insert<S>(&self, _key: &K, _cache: &crate::LightCache<K, V, S, Self>) {}

    #[inline]
    fn after_get_or_insert<S>(&self, _key: &K, _cache: &crate::LightCache<K, V, S, Self>) {}

    #[inline]
    fn after_remove<S>(&self, _key: &K, _cache: &crate::LightCache<K, V, S, Self>) {}

    #[inline]
    fn is_expired(&self, _key: &K) -> bool {
        false
    }
}