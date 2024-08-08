use super::Policy;

#[derive(Clone, Copy, Debug)]
pub struct NoopPolicy;

impl<K, V> Policy<K, V> for NoopPolicy {
    #[inline]
    fn on_get_or_insert<S>(&self, _key: &K, _cache: &crate::LightCache<K, V, S, Self>) {}

    #[inline]
    fn on_remove<S>(&self, _key: &K, _cache: &crate::LightCache<K, V, S, Self>) {}
}