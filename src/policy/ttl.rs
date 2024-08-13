use std::{
    fmt::Debug,
    hash::{BuildHasher, Hash},
    marker::PhantomData,
    sync::{Arc, Mutex, MutexGuard},
    time::{Duration, Instant},
};

use super::{
    linked_arena::{LinkedArena, LinkedNode},
    Policy, Prune,
};
use crate::LightCache;

/// A simple time-to-live policy that removes only expired keys when the ttl is exceeded
pub struct TtlPolicy<K, V> {
    inner: Arc<Mutex<TtlPolicyInner<K>>>,
    /// borrow chcker complains and requires fully qualified syntax without this which is annoying
    phantom: PhantomData<V>,
}

impl<K, V> Clone for TtlPolicy<K, V> {
    fn clone(&self) -> Self {
        TtlPolicy {
            inner: self.inner.clone(),
            phantom: self.phantom,
        }
    }
}

pub struct TtlPolicyInner<K> {
    ttl: Duration,
    arena: LinkedArena<K, TtlNode<K>>,
}

impl<K, V> TtlPolicy<K, V> {
    pub fn new(ttl: Duration) -> Self {
        TtlPolicy {
            inner: Arc::new(Mutex::new(TtlPolicyInner {
                ttl,
                arena: LinkedArena::new(),
            })),
            phantom: PhantomData,
        }
    }
}

impl<K, V> Policy<K, V> for TtlPolicy<K, V>
where
    K: Copy + Eq + Hash,
    V: Clone + Sync,
{
    type Inner = TtlPolicyInner<K>;

    #[inline]
    fn lock_inner(&self) -> MutexGuard<'_, Self::Inner> {
        self.inner.lock().unwrap()
    }

    fn get<S: BuildHasher>(&self, key: &K, cache: &LightCache<K, V, S, Self>) -> Option<V> {
        self.prune(cache);

        cache.get_no_policy(key)
    }

    fn insert<S: BuildHasher>(&self, key: K, value: V, cache: &LightCache<K, V, S, Self>) -> Option<V> {
        {
            let mut inner = self.lock_inner();
            inner.prune(cache);

            // were updating the value, so lets reset the creation time
            if let Some((idx, node)) = inner.arena.get_node_mut(&key) {
                node.creation = Instant::now();

                inner.arena.move_to_head(idx);
            } else {
                inner.arena.insert_head(key);
            }
        }

        cache.insert_no_policy(key, value)
    }

    fn remove<S: BuildHasher>(&self, key: &K, cache: &LightCache<K, V, S, Self>) -> Option<V> {
        {
            let mut inner = self.lock_inner();

            inner.prune(cache);
            inner.arena.remove_item(key);
        }

        cache.remove_no_policy(key)
    }
}

impl<K, V> Prune<K, V, TtlPolicy<K, V>> for TtlPolicyInner<K>
where
    K: Copy + Eq + Hash,
    V: Clone + Sync,
{
    fn prune<S: BuildHasher>(&mut self, cache: &LightCache<K, V, S, TtlPolicy<K, V>>) {
        while let Some(tail) = self.arena.tail {
            // saftey: we should have a valid tail index
            if unsafe { self.arena.nodes.get_unchecked(tail).should_evict(self.ttl) } {
                let (_, n) = self.arena.remove(tail);
                cache.remove_no_policy(n.item());
            } else {
                break;
            }
        }
    }
}

pub struct TtlNode<K> {
    key: K,
    creation: Instant,
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
            creation: Instant::now(),
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
            .field("creation", &self.creation)
            .field("parent", &self.parent)
            .field("child", &self.child)
            .finish()
    }
}

impl<K> TtlNode<K> {
    fn duration_since_creation(&self) -> Duration {
        Instant::now().duration_since(self.creation)
    }

    fn should_evict(&self, ttl: Duration) -> bool {
        self.duration_since_creation() > ttl
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

    fn insert_n<S>(cache: &LightCache<i32, i32, S, TtlPolicy<i32, i32>>, n: usize)
    where
        S: BuildHasher,
    {
        for i in 0..n {
            cache.insert(i as i32, i as i32);
        }
    }

    fn cache<K, V>(ttl: Duration) -> LightCache<K, V, DefaultHashBuilder, TtlPolicy<K, V>>
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
    fn test_basic_scenario_2() {
        let cache = cache::<i32, i32>(duration_seconds(1));

        insert_n(&cache, 10);

        cache.remove(&0);

        sleep_seconds(2);

        insert_n(&cache, 2);

        assert_eq!(cache.len(), 2);
    }
}
