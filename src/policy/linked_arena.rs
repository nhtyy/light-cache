
use hashbrown::HashMap;

use crate::LightCache;
use super::Policy;

use std::hash::{BuildHasher, Hash};

/// A doubly linked list arena
///
/// Useful for creating various caching policies
pub(crate) struct LinkedArena<I, N> {
    pub(crate) idx_of: HashMap<I, usize>,
    pub(crate) nodes: Vec<N>,
    pub(crate) head: Option<usize>,
    pub(crate) tail: Option<usize>,
    _phantom: std::marker::PhantomData<I>,
}

impl<I, N> LinkedArena<I, N> {
    pub fn new() -> Self {
        LinkedArena {
            idx_of: HashMap::new(),
            nodes: Vec::new(),
            head: None,
            tail: None,
            _phantom: std::marker::PhantomData,
        }
    }
}

pub trait LinkedNode<I>
where
    Self: Sized,
    I: Copy + Hash + Eq,
{
    fn new(item: I, parent: Option<usize>, child: Option<usize>) -> Self;
    fn item(&self) -> &I;

    fn prev(&self) -> Option<usize>;
    fn next(&self) -> Option<usize>;

    fn set_prev(&mut self, parent: Option<usize>);
    fn set_next(&mut self, child: Option<usize>);

    fn should_evict<V, S, P>(&self, cache: &LightCache<I, V, S, P>) -> bool
    where
        V: Clone + Sync,
        S: BuildHasher,
        P: Policy<I, V, Node = Self>,
    {
        cache.policy.is_expired(self)
    }
}

impl<I, N> LinkedArena<I, N>
where
    N: LinkedNode<I>,
    I: Copy + Hash + Eq,
{
    // Insert a new node at the front of the list
    pub(crate) fn insert_head(&mut self, key: I) {
        debug_assert!(self.idx_of.get(&key).is_none());

        let new_head = self.nodes.len();
        self.idx_of.insert(key, new_head);

        if let Some(old_head) = self.head {
            self.nodes.push(N::new(key, None, Some(old_head)));

            // saftey: we should have a valid old head idx
            unsafe {
                self.nodes
                    .get_unchecked_mut(old_head)
                    .set_prev(Some(new_head));
            }
        } else {
            self.nodes.push(N::new(key, None, None));
            self.tail = Some(new_head);
        }

        self.head = Some(new_head);
    }

    /// Ensures the node at idx has the correct parent and child relationships
    ///
    /// # Note:
    /// noop if the idx is out of bounds
    fn relink(&mut self, idx: usize) {
        if let Some(node) = self.nodes.get(idx) {
            let prev = node.prev();
            let next = node.next();

            if let Some(parent) = prev {
                // saftey: the parent indexes are always valid in arena
                unsafe {
                    self.nodes.get_unchecked_mut(parent).set_next(Some(idx));
                }
            } else {
                self.head = Some(idx);
            }

            if let Some(child) = next {
                // saftey: the child indexes are always valid in arena
                unsafe {
                    self.nodes.get_unchecked_mut(child).set_prev(Some(idx));
                }
            } else {
                self.tail = Some(idx);
            }
        }
    }

    /// Remove the node at idx from the list
    /// passing through any parent/child relationships
    ///
    /// # Panics
    /// if idx is out of bounds
    fn unlink(&mut self, idx: usize) {
        let node = self.nodes.get(idx).expect("Invalid index to unlink");
        let parent = node.prev();
        let child = node.next();

        // saftey: we should have a valid parent and child idx
        if let Some(parent) = parent {
            unsafe {
                self.nodes.get_unchecked_mut(parent).set_next(child);
            }
        } else {
            self.head = child;
        }

        if let Some(child) = child {
            unsafe {
                self.nodes.get_unchecked_mut(child).set_prev(parent);
            }
        } else {
            self.tail = parent;
        }
    }
}

/// All methods here should never be able to create an invalid state on the list
impl<I, N> LinkedArena<I, N>
where
    N: LinkedNode<I>,
    I: Copy + Hash + Eq,
{
    #[allow(unused)]
    /// Move the node at idx to the front of the list
    ///
    /// # Panics
    /// Panics if new_head is out of bounds
    pub(crate) fn move_to_head_key(&mut self, key: &I) {
        if let Some(new_head_idx) = self.idx_of.get(key).copied() {
            self.unlink(new_head_idx);

            // saftey: unlink checks that the idx is valid
            let node = unsafe { self.nodes.get_unchecked_mut(new_head_idx) };
            node.set_next(self.head);
            node.set_prev(None);

            if let Some(old) = self.head.replace(new_head_idx) {
                unsafe {
                    // unwrap: we should have a valid old head idx
                    self.nodes.get_unchecked_mut(old).set_prev(Some(new_head_idx));
                }
            }
        } else {
            panic!("Invalid key to move to head on LinkedArena");
        }
    }

    pub(crate) fn move_to_head(&mut self, new_head: usize) {
        self.unlink(new_head);

        // saftey: unlink checks that the idx is valid
        let node = unsafe { self.nodes.get_unchecked_mut(new_head) };
        node.set_next(self.head);
        node.set_prev(None);

        if let Some(old) = self.head.replace(new_head) {
            unsafe {
                // unwrap: we should have a valid old head idx
                self.nodes.get_unchecked_mut(old).set_prev(Some(new_head));
            }
        }
    }

    pub(crate) fn get_node_mut(&mut self, key: &I) -> Option<(usize, &mut N)> {
        if let Some(idx) = self.idx_of.get(key).copied() {
            Some((idx, unsafe { self.nodes.get_unchecked_mut(idx) }))
        } else {
            None
        }
    }

    /// Remove the node from the given index, updating the start or end bounds as needed
    /// and returning the removed node.
    ///
    /// This method will also call remove on the cache even if already done
    ///
    /// # Panics
    /// IF idx is out of bounds
    pub(crate) fn remove_item(&mut self, item: &I) -> Option<(usize, N)> {
        if let Some(idx) = self.idx_of.get(item).copied() {
            Some(self.remove(idx))
        } else {
            None
        }
    }

    /// Remove the node from the given index, updating the start or end bounds as needed
    /// and returning the removed node.
    ///
    /// This method will also call remove on the cache even if already done
    ///
    /// # Panics
    /// If idx is out of bounds
    pub(crate) fn remove(&mut self, idx: usize) -> (usize, N) {
        // panics if the idx is out of bounds
        self.unlink(idx);

        let removed = self.nodes.swap_remove(idx);
        self.idx_of.remove(removed.item());

        let len = self.nodes.len();
        // if the last element was just removed than this index will be out of bounds
        // and theres nothing to relink cause nothing was moved
        if idx != len {
            // unwrap: we should have a valid item at idx now and we should have a valid idx for the item
            *self
                .idx_of
                .get_mut(self.nodes.get(idx).unwrap().item())
                .unwrap() = idx;

            self.relink(idx);
        }

        (len, removed)
    }

    /// # Note
    ///
    /// This function assumes the list is sorted and that any expired nodes are at the end of the list
    pub(crate) fn clear_expired<V, S, P>(&mut self, cache: &LightCache<I, V, S, P>)
    where
        V: Clone + Sync,
        S: BuildHasher,
        P: Policy<I, V, Node = N>,
    {
        loop {
            if let Some(tail) = self.tail {
                // saftey: we should have a valid tail index
                if unsafe { self.nodes.get_unchecked(tail).should_evict(cache) } {
                    let (_, n) = self.remove(tail);
                    cache.remove_no_policy(n.item());
                } else {
                    break;
                }
            } else {
                break;
            }
        }
    }
}
