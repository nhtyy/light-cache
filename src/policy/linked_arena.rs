use std::hash::{BuildHasher, Hash};

use hashbrown::HashMap;

use crate::LightCache;

use super::{Expiry, Policy};

/// A doubly linked list arena
/// 
/// Useful for creating various caching policies
pub(crate) struct LinkedArena<I, N, E> {
    pub(crate) expiry: E,
    pub(crate) idx_of: HashMap<I, usize>,
    pub(crate) nodes: Vec<N>,
    pub(crate) head: Option<usize>,
    pub(crate) tail: Option<usize>,
    _phantom: std::marker::PhantomData<I>,
}

impl<I, N, E> LinkedArena<I, N, E> {
    pub fn new(expiry: E) -> Self {
        LinkedArena {
            expiry,
            idx_of: HashMap::new(),
            nodes: Vec::new(),
            head: None,
            tail: None,
            _phantom: std::marker::PhantomData,
        }
    }
}

pub trait LinkedNode<I, E>
where
    Self: Sized,
    I: Copy + Hash + Eq,
{
    fn new(item: I, parent: Option<usize>, child: Option<usize>) -> Self;
    fn item(&self) -> &I;

    fn parent(&self) -> Option<usize>;
    fn child(&self) -> Option<usize>;

    fn set_parent(&mut self, parent: Option<usize>);
    fn set_child(&mut self, child: Option<usize>);

    fn should_evict(&self, arena: &LinkedArena<I, Self, E>) -> bool;
}

impl<I, N, E> LinkedArena<I, N, E>
where
    N: LinkedNode<I, E>,
    I: Copy + Hash + Eq,
{
    // Insert a new node at the front of the list
    pub(crate) fn insert_head(&mut self, key: I) {
        debug_assert!(self.idx_of.get(&key).is_none());

        let new_start = self.nodes.len();
        self.idx_of.insert(key, new_start);

        if let Some(old_start) = self.head {
            self.nodes.push(N::new(key, None, Some(old_start)));

            // unwrap: we should have a valid old head idx
            self.nodes
                .get_mut(old_start)
                .unwrap()
                .set_parent(Some(new_start));
        } else {
            self.nodes.push(N::new(key, None, None));
            self.tail = Some(new_start);
        }

        self.head = Some(new_start);
    }

    /// Ensures the node at idx has the correct parent and child relationships
    ///
    /// # Note:
    /// noop if the idx is out of bounds
    pub(crate) fn relink(&mut self, idx: usize) {
        if let Some(node) = self.nodes.get(idx) {
            let parent = node.parent();
            let child = node.child();

            if let Some(parent) = parent {
                self.nodes.get_mut(parent).unwrap().set_child(Some(idx));
            } else {
                self.head = Some(idx);
            }

            if let Some(child) = child {
                self.nodes.get_mut(child).unwrap().set_parent(Some(idx));
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
    pub(crate) fn unlink(&mut self, idx: usize) {
        let node = self.nodes.get(idx).unwrap();
        let parent = node.parent();
        let child = node.child();

        // unwraps: we should have a valid parent and child idx
        if let Some(parent) = parent {
            self.nodes.get_mut(parent).unwrap().set_child(child);
        } else {
            self.head = child;
        }

        if let Some(child) = child {
            self.nodes.get_mut(child).unwrap().set_parent(parent);
        } else {
            self.tail = parent;
        }
    }

    /// Move the node at idx to the front of the list
    ///
    /// # Panics
    /// Panics if new_head is out of bounds
    pub(crate) fn move_to_head(&mut self, new_head: usize) {
        self.unlink(new_head);

        let node = self.nodes.get_mut(new_head).unwrap();
        node.set_child(self.head);
        node.set_parent(None);

        if let Some(old) = self.head.replace(new_head) {
            // unwrap: we should have a valid old head idx
            self.nodes.get_mut(old).unwrap().set_parent(Some(new_head));
        }
    }
}

impl<I, N, E> LinkedArena<I, N, E>
where
    N: LinkedNode<I, E>,
    I: Copy + Hash + Eq,
    E: Expiry<N>,
{
    /// Remove the node from the given index, updating the start or end bounds as needed
    /// and returning the removed node.
    ///
    /// This method will also call remove on the cache even if already done
    ///
    /// # Panics
    /// IF idx is out of bounds
    pub(crate) fn remove_item<V, S, P>(&mut self, item: &I, cache: &LightCache<I, V, S, P>) -> Option<(usize, N)>
    where
        V: Clone + Sync,
        S: BuildHasher,
        P: Policy<I, V, Expiry = E>,
    {
        if let Some(idx) = self.idx_of.get(item).copied() {
            Some(self.remove(idx, cache))
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
    pub(crate) fn remove<V, S, P>(&mut self, idx: usize, cache: &LightCache<I, V, S, P>) -> (usize, N)
    where
        V: Clone + Sync,
        S: BuildHasher,
        P: Policy<I, V, Expiry = E>,
    {
        self.unlink(idx);

        let removed = self.nodes.swap_remove(idx);
        self.idx_of.remove(removed.item());
        cache.remove_no_policy(removed.item());

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

    pub(crate) fn clear_expired<V, S, P>(&mut self, cache: &LightCache<I, V, S, P>)
    where
        V: Clone + Sync,
        S: BuildHasher,
        P: Policy<I, V, Expiry = E>,
    {
        loop {
            if let Some(tail) = self.tail {
                if self.nodes.get(tail).unwrap().should_evict(self) {
                    self.remove(tail, cache);
                } else {
                    break;
                }
            } else {
                break;
            }
        }
    }
}
