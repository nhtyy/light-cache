use std::{
    fmt::Debug,
    hash::{BuildHasher, Hash},
    marker::PhantomData,
    sync::{Arc, Mutex, MutexGuard},
    time::{Duration, Instant},
    cmp::Reverse,
};

use priority_queue::PriorityQueue;

use super::{
    linked_arena::{LinkedArena, LinkedNode},
    Policy, Prune,
};
use crate::LightCache;

/// A simple least-recently-used policy that removes entries based on capacity and usage recency
pub struct LruPolicy<K, V> {
    inner: Arc<Mutex<LruPolicyInner<K>>>,
    /// borrow chcker complains and requires fully qualified syntax without this which is annoying
    phantom: PhantomData<V>,
}

impl<K, V> Clone for LruPolicy<K, V> {
    fn clone(&self) -> Self {
        LruPolicy {
            inner: self.inner.clone(),
            phantom: self.phantom,
        }
    }
}

pub struct LruPolicyInner<K> {
    capacity: usize,
    arena: LinkedArena<K, LruNode<K>>,
    expiring: Option<(Duration, PriorityQueue<K, Reverse<Instant>>)>,
}

impl<K: Hash + Eq, V> LruPolicy<K, V> {
    pub fn new(capacity: usize, node_lifetime: Option<Duration>) -> Self {
        if let Some(duration) = node_lifetime {
            LruPolicy {
                inner: Arc::new(Mutex::new(LruPolicyInner {
                    capacity,
                    arena: LinkedArena::new(),
                    expiring: Some((duration, PriorityQueue::new())),
                })),
                phantom: PhantomData,
            }
        } else {
            LruPolicy {
                inner: Arc::new(Mutex::new(LruPolicyInner {
                    capacity,
                    arena: LinkedArena::new(),
                    expiring: None,
                })),
                phantom: PhantomData,
            }
        }
        
    }
}

impl<K, V> Policy<K, V> for LruPolicy<K, V>
where
    K: Copy + Eq + Hash,
    V: Clone + Sync,
{
    type Inner = LruPolicyInner<K>;

    #[inline]
    fn lock_inner(&self) -> MutexGuard<'_, Self::Inner> {
        self.inner.lock().unwrap()
    }

    fn get<S: BuildHasher>(&self, key: &K, cache: &LightCache<K, V, S, Self>) -> Option<V> {
        {
            let mut inner = self.lock_inner();
            inner.prune(cache);

            if let Some((idx, node)) = inner.arena.get_node_mut(&key) {
                let mut inner = self.lock_inner();
                inner.arena.move_to_head(idx);
            }
        }

        cache.get_no_policy(key)
    }

    fn insert<S: BuildHasher>(&self, key: K, value: V, cache: &LightCache<K, V, S, Self>) -> Option<V> {
        {
            let mut inner = self.lock_inner();
            inner.prune(cache);

            // were updating the value, so lets reset the creation time
            if let Some((idx, node)) = inner.arena.get_node_mut(&key) {
                inner.arena.move_to_head(idx);
            } else {
                inner.arena.insert_head(key);
            }

            if let Some((_, pq)) = inner.expiring.as_mut() {
                pq.push(key, Reverse(Instant::now()));
            }

            inner.evict(cache);
        }
        
        cache.insert_no_policy(key, value)
    }

    fn remove<S: BuildHasher>(&self, key: &K, cache: &LightCache<K, V, S, Self>) -> Option<V> {
        {
            let mut inner = self.lock_inner();
            inner.arena.remove_item(key);

            if let Some((_, pq)) = inner.expiring.as_mut() {
                pq.remove(key);
            }
        }

        cache.remove_no_policy(key)
    }
}

impl<K, V> Prune<K, V, LruPolicy<K, V>> for LruPolicyInner<K>
where
    K: Copy + Eq + Hash,
    V: Clone + Sync,
{
    fn prune<S: BuildHasher>(&mut self, cache: &LightCache<K, V, S, LruPolicy<K, V>>) {
        while let Some((idx, _)) = self.arena.tail() {
            if self.arena.len() > self.capacity {
                let (_, n) = self.arena.remove(idx);
                cache.remove_no_policy(n.item());
                if let Some((_, pq)) = self.expiring.as_mut() {
                    pq.remove(&n.key);
                }
            } else {
                break;
            }
        }
        if let Some((lifetime, pq)) = self.expiring.as_mut() {
            while let Some(top) = pq.peek() {
                let now = Instant::now();
                if now.duration_since(top.1.0) > *lifetime {
                    self.arena.remove_item(top.0);
                    cache.remove_no_policy(top.0);
                    pq.pop();
                } else {
                    break;
                }
            }
        }
    }
}

impl<K: Copy + Eq + Hash> LruPolicyInner<K> {
    fn evict<S: BuildHasher, V: Clone + Sync>(&mut self, cache: &LightCache<K, V, S, LruPolicy<K, V>>) {
        while let Some((idx, _)) = self.arena.tail() {
            if self.arena.len() > self.capacity {
                let (_, n) = self.arena.remove(idx);
                cache.remove_no_policy(n.item());
                if let Some((_, pq)) = self.expiring.as_mut() {
                    pq.remove(&n.key);
                }
            } else {
                break;
            }
        }
    }
}

pub struct LruNode<K> {
    key: K,
    prev: Option<usize>,
    next: Option<usize>,
}

impl<K> LinkedNode<K> for LruNode<K>
where
    K: Copy + Eq + Hash,
{
    fn new(key: K, prev: Option<usize>, next: Option<usize>) -> Self {
        LruNode {
            key,
            prev,
            next,
        }
    }

    fn item(&self) -> &K {
        &self.key
    }

    fn prev(&self) -> Option<usize> {
        self.prev
    }

    fn next(&self) -> Option<usize> {
        self.next
    }

    fn set_prev(&mut self, prev: Option<usize>) {
        self.prev = prev;
    }

    fn set_next(&mut self, next: Option<usize>) {
        self.next = next;
    }
}

impl<K> Debug for LruNode<K> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LruNode")
            .field("prev", &self.prev)
            .field("next", &self.next)
            .finish()
    }
}

#[cfg(test)]
mod test {
    use hashbrown::hash_map::DefaultHashBuilder;

    use super::*;

    fn duration_seconds(seconds: u64) -> Duration {
        Duration::from_secs(seconds)
    }

    fn sleep_seconds(seconds: u64) {
        std::thread::sleep(duration_seconds(seconds));
    }

    fn insert_n<S>(cache: &LightCache<i32, i32, S, LruPolicy<i32, i32>>, n: usize)
    where
        S: BuildHasher,
    {
        for i in 0..n {
            cache.insert(i as i32, i as i32);
        }
    }

    fn cache<K, V>(capacity: usize, lifetime: Duration) -> LightCache<K, V, DefaultHashBuilder, LruPolicy<K, V>>
    where
        K: Copy + Eq + Hash,
        V: Clone + Sync,
    {
        LightCache::from_parts(LruPolicy::new(capacity, Some(lifetime)), Default::default())
    }

    #[test]
    /// Insert 5 keys, and insert 2 more keys
    /// this will leave only the last 2 items inserted
    fn test_basic_scenario_1() {
        let cache = cache::<i32, i32>(5, duration_seconds(1));

        insert_n(&cache, 5);

        sleep_seconds(2);

        insert_n(&cache, 2);
       
        assert_eq!(cache.len(), 2);
        let policy = cache.policy().lock_inner();

        assert_eq!(policy.arena.nodes.len(), 2);
        assert_eq!(policy.arena.head, Some(1));
        assert_eq!(policy.arena.tail, Some(0));
    }

    #[test]
    fn test_basic_scenario_2() {
        let cache = cache::<i32, i32>(5, duration_seconds(2));

        insert_n(&cache, 10);
        cache.remove(&8);
     
        assert_eq!(cache.len(), 4);
    }
}
