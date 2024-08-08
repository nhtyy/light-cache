use hashbrown::HashMap;
use std::{
    fmt::Debug,
    hash::{BuildHasher, Hash},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use crate::LightCache;

use super::Policy;

/// A simple time-to-live policy that removes only expired keys when the ttl is exceeded
/// Kind of like an unbounded LRU
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

pub struct TtlPolicyInner<K> {
    ttl: Duration,
    idx_of: HashMap<K, usize>,
    keys: Vec<TtlNode<K>>,
    start_idx: Option<usize>,
    end_idx: Option<usize>,
}

impl<K> TtlPolicy<K> {
    pub fn new(ttl: Duration) -> Self {
        TtlPolicy {
            inner: Arc::new(Mutex::new(TtlPolicyInner {
                ttl,
                idx_of: HashMap::new(),
                keys: Vec::new(),
                start_idx: None,
                end_idx: None,
            })),
        }
    }
}

impl<K: Copy + Eq + Hash> TtlPolicyInner<K> {
    fn try_to_clear<V, S>(&mut self, cache: &LightCache<K, V, S, TtlPolicy<K>>)
    where
        V: Clone + Sync,
        S: BuildHasher,
    {
        if let Some(starting_end_idx) = self.end_idx {
            let mut new_end_idx = starting_end_idx;

            loop {
                let end = self.keys.get(new_end_idx).unwrap();
                if end.duration_since_last_touched() > self.ttl {
                    if let Some(parent) = end.parent {
                        let len = self.remove_node(new_end_idx, cache);
    
                        // if the parent node is the end of the buffer we
                        // know its gonna get moved to the current slot so just continue the loop
                        if parent != len {
                            new_end_idx = parent;
                        }
                    } else {
                        self.end_idx = None;
                        self.start_idx = None;
                        self.keys.clear();
                        self.idx_of.clear();
    
                        return;
                    }
                } else {
                    break;
                }           
            }

            if new_end_idx != starting_end_idx {
                self.end_idx = Some(new_end_idx);
            }
        }
    }

    // removes the node at the given index, without updating the start or end idx
    // returns the new length of the keys vec
    fn remove_node<V, S>(&mut self, idx: usize, cache: &LightCache<K, V, S, TtlPolicy<K>>) -> usize
    where
        V: Clone + Sync,
        S: BuildHasher,
    {
        self.unlink_node(idx);

        let removed = self.keys.swap_remove(idx);
        self.idx_of.remove(&removed.key);
        cache.remove(&removed.key);

        let len = self.keys.len();
        if self.keys.len() > 1 {
            self.relink_node(idx);
        }

        len
    }
}

impl<K: Copy + Eq + Hash> TtlPolicyInner<K> {
    // Insert a new node at the front of the list
    // cheaper than inserting, and relinking front
    fn insert_new_key(&mut self, key: K) {
        debug_assert!(self.idx_of.get(&key).is_none());

        let new_start = self.keys.len();
        self.idx_of.insert(key, new_start);

        if let Some(old_start) = self.start_idx {
            self.keys.push(TtlNode::new(key, None, Some(old_start)));
            self.keys[old_start].parent = Some(new_start);
        } else {
            self.keys.push(TtlNode::new(key, None, None));
            self.end_idx = Some(new_start);
        }

        self.start_idx = Some(new_start);
    }

    /// Ensures the node at idx has the correct parent and child relationships
    fn relink_node(&mut self, idx: usize) {
        if let Some(node) = self.keys.get(idx) {
            let parent = node.parent;
            let child = node.child;

            if let Some(parent) = parent {
                self.keys[parent].child = Some(idx);
            } else {
                self.start_idx = Some(idx);
            }

            if let Some(child) = child {
                self.keys[child].parent = Some(idx);
            }
        }
    }

    /// If you think of the nodes as a doubly linked list, this function will remove the node at idx
    /// and pass the parent and child relationships to each other if they exist
    /// # Panics
    /// If the node at idx doesnt exist
    fn unlink_node(&mut self, idx: usize) {
        let node = self.keys.get(idx).unwrap();
        let parent = node.parent;
        let child = node.child;

        if let Some(parent) = parent {
            self.keys[parent].child = child;
        }

        if let Some(child) = child {
            self.keys[child].parent = parent;
        }
    }

    fn link_new_front(&mut self, new: usize) {
        self.unlink_node(new);

        let node = self.keys.get_mut(new).unwrap();
        node.child = self.start_idx;
        node.parent = None;
        node.last_touched = Instant::now();

        if let Some(old) = self.start_idx.replace(new) {
            self.keys[old].parent = Some(new);
        }
    }
}

impl<K, V> Policy<K, V> for TtlPolicy<K>
where
    K: Copy + Eq + Hash,
    V: Clone + Sync,
{
    fn after_get_or_insert<S: BuildHasher>(&self, key: &K, cache: &LightCache<K, V, S, Self>) {
        let mut lock = self.inner.lock().unwrap();
        if let Some(new_front) = lock.idx_of.get(key).copied() {
            // store the current parent of the new front since this will be the new end
            // we can just skip it if its already the front (no parent)
            if let Some(new_front_old_parent) = lock.keys[new_front].parent {
                lock.link_new_front(new_front);

                if new_front == lock.end_idx.unwrap() {
                    lock.end_idx = Some(new_front_old_parent);
                }
            }
        } else {
            lock.insert_new_key(*key);
        }

        lock.try_to_clear(cache);
    }

    fn after_remove<S>(&self, key: &K, cache: &LightCache<K, V, S, Self>) {}
}

struct TtlNode<K> {
    key: K,
    last_touched: Instant,
    parent: Option<usize>,
    child: Option<usize>,
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

        assert_eq!(policy.idx_of.keys().len(), 2);
        assert_eq!(policy.start_idx, Some(1));
        assert_eq!(policy.end_idx, Some(0));
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
    }
}
