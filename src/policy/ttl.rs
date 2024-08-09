use std::{
    fmt::Debug,
    hash::{BuildHasher, Hash},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use super::{
    linked_arena::{LinkedArena, LinkedNode},
    Expiry, Policy,
};
use crate::LightCache;

/// A simple time-to-live policy that removes only expired keys when the ttl is exceeded
pub struct TtlPolicy<K> {
    inner: Arc<Mutex<TtlPolicyInner<K>>>,
}

impl<K> Clone for TtlPolicy<K> {
    fn clone(&self) -> Self {
        TtlPolicy {
            inner: self.inner.clone(),
        }
    }
}

pub struct TtlExpiry {
    ttl: Duration,
}

pub struct TtlPolicyInner<K> {
    arena: LinkedArena<K, TtlNode<K>, TtlExpiry>,
}

impl<K> TtlPolicy<K> {
    pub fn new(ttl: Duration) -> Self {
        TtlPolicy {
            inner: Arc::new(Mutex::new(TtlPolicyInner {
                arena: LinkedArena::new(TtlExpiry { ttl }),
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
    type Expiry = TtlExpiry;

    fn after_get_or_insert<S: BuildHasher>(&self, key: &K, cache: &LightCache<K, V, S, Self>) {
        let mut policy = self.inner.lock().unwrap();

        if let Some(idx) = policy.arena.idx_of.get(key).copied() {
            let node = &mut policy.arena.nodes[idx];
            node.last_touched = Instant::now();

            policy.arena.move_to_head(idx);
        } else {
            policy.arena.insert_head(*key);
        }

        policy.arena.clear_expired(cache);
    }

    fn after_remove<S: BuildHasher>(&self, key: &K, cache: &LightCache<K, V, S, Self>) {
        let mut policy = self.inner.lock().unwrap();

        if let None = policy.arena.remove_item(key, cache) {
            unreachable!("Key should have been in the cache");
        }

        policy.arena.clear_expired(cache);
    }
}

pub struct TtlNode<K> {
    key: K,
    last_touched: Instant,
    parent: Option<usize>,
    child: Option<usize>,
}

impl<K> Expiry<TtlNode<K>> for TtlExpiry {
    fn is_expired(&self, node: &TtlNode<K>) -> bool {
        node.duration_since_last_touched() > self.ttl
    }
}

impl<K> LinkedNode<K, TtlExpiry> for TtlNode<K>
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

    fn parent(&self) -> Option<usize> {
        self.parent
    }

    fn child(&self) -> Option<usize> {
        self.child
    }

    fn set_parent(&mut self, parent: Option<usize>) {
        self.parent = parent;
    }

    fn set_child(&mut self, child: Option<usize>) {
        self.child = child;
    }

    fn should_evict(&self, arena: &LinkedArena<K, Self, TtlExpiry>) -> bool {
        arena.expiry.is_expired(self)
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
    fn new(key: K, parent: Option<usize>, child: Option<usize>) -> Self {
        TtlNode {
            key,
            last_touched: Instant::now(),
            parent,
            child,
        }
    }

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
