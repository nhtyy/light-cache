use crate::LightCache;
use std::hash::{BuildHasher, Hash};

/// The [NoPolicy] trait is used for external policy implementations.
/// 
/// These methods are explicity kept seperate outside the crate to not pollute the API.
pub trait NoPolicy<K, V, P> {
    fn get_no_policy(&self, key: &K) -> Option<V>;

    fn insert_no_policy(&self, key: K, value: V) -> Option<V>;

    fn remove_no_policy(&self, key: &K) -> Option<V>;

    fn policy(&self) -> &P;
}

impl<K, V, S, P> NoPolicy<K, V, P> for LightCache<K, V, S, P>
where
    K: Eq + Hash + Copy,
    V: Clone + Sync,
    S: BuildHasher,
{
    #[inline]
    fn get_no_policy(&self, key: &K) -> Option<V> {
        self.get_no_policy(key)
    }

    #[inline]
    fn insert_no_policy(&self, key: K, value: V) -> Option<V> {
        self.insert_no_policy(key, value)
    }

    #[inline]
    fn remove_no_policy(&self, key: &K) -> Option<V> {
        self.remove_no_policy(key)
    }

    #[inline]
    fn policy(&self) -> &P {
        self.policy()
    }
}