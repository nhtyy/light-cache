use std::{
    fmt::Debug,
    hash::{BuildHasher, Hash},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use super::{
    linked_arena::{LinkedArena, LinkedNode},
    Policy,
};
use crate::{cache::GetOrInsertFuture, LightCache};

/// A simple time-to-live policy that removes only expired keys when the ttl is exceeded
///
/// If a value is accessed it will be moved to the front of the cache and thus not be removed until after the ttl
/// If the values may become stale before the ttl is exceeded, consider using a refresh policy
pub struct TtlPolicy<K> {
    ttl: Duration,
    inner: Arc<Mutex<TtlPolicyInner<K>>>,
}

impl<K> Clone for TtlPolicy<K> {
    fn clone(&self) -> Self {
        TtlPolicy {
            ttl: self.ttl,
            inner: self.inner.clone(),
        }
    }
}

pub struct TtlPolicyInner<K> {
    arena: LinkedArena<K, TtlNode<K>>,
}

impl<K> TtlPolicy<K> {
    pub fn new(ttl: Duration) -> Self {
        TtlPolicy {
            ttl,
            inner: Arc::new(Mutex::new(TtlPolicyInner {
                arena: LinkedArena::new(),
            })),
        }
    }
}

impl<K, V> Policy<K, V> for TtlPolicy<K>
where
    K: Copy + Eq + Hash,
    V: Clone + Sync,
{
    type Node = TtlNode<K>;

    fn get<S: BuildHasher>(&self, key: &K, cache: &LightCache<K, V, S, Self>) -> Option<V> {
        let mut policy = self.inner.lock().unwrap();

        policy.arena.clear_expired(cache);
        if let Some((idx, node)) = policy.arena.get_node_mut(key) {
            node.last_touched = Instant::now();

            policy.arena.move_to_head(idx);
        }

        cache.get_no_policy(key)
    }

    fn insert<S: BuildHasher>(&self, key: K, value: V, cache: &LightCache<K, V, S, Self>) {
        let mut policy = self.inner.lock().unwrap();

        policy.arena.clear_expired(cache);
        if let Some((idx, node)) = policy.arena.get_node_mut(&key) {
            node.last_touched = Instant::now();

            policy.arena.move_to_head(idx);
        } else {
            policy.arena.insert_head(key);
        }

        cache.insert_no_policy(key, value);
    }

    fn remove<S: BuildHasher>(&self, key: &K, cache: &LightCache<K, V, S, Self>) -> Option<V> {
        let mut policy = self.inner.lock().unwrap();

        // theres a chance were removing when this key is expired so lets just take it now for safekeeping
        let v = cache.remove_no_policy(key);

        policy.arena.clear_expired(cache);
        policy.arena.remove_item(key);

        v
    }

    fn get_or_insert<'a, S, F, Fut>(
        &self,
        key: K,
        cache: &'a LightCache<K, V, S, Self>,
        init: F,
    ) -> GetOrInsertFuture<'a, K, V, S, F, Fut>
    where
        S: BuildHasher,
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = V>,
    {
        let mut policy = self.inner.lock().unwrap();

        policy.arena.clear_expired(cache);
        if let Some((idx, node)) = policy.arena.get_node_mut(&key) {
            node.last_touched = Instant::now();

            policy.arena.move_to_head(idx);
        } else {
            policy.arena.insert_head(key);
        }

        cache.get_or_insert_no_policy(key, init)
    }

    fn is_expired(&self, node: &TtlNode<K>) -> bool {
        self.ttl < node.duration_since_last_touched()
    }
}

pub struct TtlNode<K> {
    key: K,
    last_touched: Instant,
    parent: Option<usize>,
    child: Option<usize>,
}

impl<K> LinkedNode<K> for TtlNode<K>
where
    K: Copy + Eq + Hash,
{
    fn new(key: K, parent: Option<usize>, child: Option<usize>) -> Self {
        TtlNode {
            key,
            last_touched: Instant::now(),
            parent,
            child,
        }
    }

    fn item(&self) -> &K {
        &self.key
    }

    fn prev(&self) -> Option<usize> {
        self.parent
    }

    fn next(&self) -> Option<usize> {
        self.child
    }

    fn set_prev(&mut self, parent: Option<usize>) {
        self.parent = parent;
    }

    fn set_next(&mut self, child: Option<usize>) {
        self.child = child;
    }
}

impl<K> Debug for TtlNode<K> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TtlNode")
            .field("last_touched", &self.last_touched)
            .field("parent", &self.parent)
            .field("child", &self.child)
            .finish()
    }
}

impl<K> TtlNode<K> {
    fn duration_since_last_touched(&self) -> Duration {
        Instant::now().duration_since(self.last_touched)
    }
}

#[cfg(test)]
mod test {
    use hashbrown::hash_map::DefaultHashBuilder;

    use super::*;
    use std::time::Duration;

    fn duration_seconds(seconds: u64) -> Duration {
        Duration::from_secs(seconds)
    }

    fn sleep_seconds(seconds: u64) {
        std::thread::sleep(duration_seconds(seconds));
    }

    fn insert_n<S>(cache: &LightCache<i32, i32, S, TtlPolicy<i32>>, n: usize)
    where
        S: BuildHasher,
    {
        for i in 0..n {
            cache.insert(i as i32, i as i32);
        }
    }

    fn cache<K, V>(ttl: Duration) -> LightCache<K, V, DefaultHashBuilder, TtlPolicy<K>>
    where
        K: Copy + Eq + Hash,
        V: Clone + Sync,
    {
        LightCache::from_parts(TtlPolicy::new(ttl), Default::default())
    }

    #[test]
    /// Insert 5 keys, wait until expiry and insert 2 more keys
    /// this will remove items from the fron tof the cache
    fn test_basic_scenario_1() {
        let cache = cache::<i32, i32>(duration_seconds(1));

        insert_n(&cache, 5);

        sleep_seconds(1);

        insert_n(&cache, 2);

        // 1 should be removed by now
        assert_eq!(cache.len(), 2);
        let policy = cache.policy.inner.lock().unwrap();

        assert_eq!(policy.arena.nodes.len(), 2);
        assert_eq!(policy.arena.head, Some(1));
        assert_eq!(policy.arena.tail, Some(0));
    }

    #[test]
    /// Insert 5 keys and then get 2 of them in the front of them halfway through, insert 2 more keys
    /// wait for the full length and insert another key this should remove some keys in the middle of the buffer
    fn test_basic_scenario_2() {
        let cache = cache::<i32, i32>(duration_seconds(2));

        insert_n(&cache, 5);

        sleep_seconds(1);

        cache.get(&2);
        cache.get(&3);

        sleep_seconds(1);

        insert_n(&cache, 2);

        let policy = cache.policy.inner.lock().unwrap();
        assert_eq!(policy.arena.nodes.len(), 4);
    }

    #[test]
    fn test_basic_scenario_3() {
        let cache = cache::<i32, i32>(duration_seconds(1));

        insert_n(&cache, 10);

        cache.remove(&0);

        sleep_seconds(2);

        insert_n(&cache, 2);

        assert_eq!(cache.len(), 2);
    }
}
