//! A simple, fast and concurrent cache designed for async rust.
//! 
//! # Quick Start
//! The easiest way to get started with LightCache is to use [`crate::RefreshCache`]
//! 
//! # Policy Implementers
//! LightCache is designed to be flexible and allow for custom policies to be implemented.
//! If you want to implement your own policy, you should implement the [`crate::Policy`] trait.
//! And you must use the [`cache::NoPolicy`] trait in your implementation to access the policy free cache methods.
//! Using [`LightCache::get`] or any other method that doesnt end in `_no_policy` will cause cause an infinite loop.
//! 
//! Another thing to possibly note is that any task using async insertion methods ([`LightCache::get_or_insert`], [`LightCache::get_or_try_insert`], etc)
//! will always first call [`Policy::get`] but only the task that actually inserts the value will call [`Policy::insert`].

pub mod cache;
#[doc(inline)]
pub use cache::LightCache;

/// The underlying map used by LightCache. Desinged to be fast and non-blocking for conncurrent r/w.
pub mod map;
#[doc(inline)]
pub use map::LightMap;

/// A policy augments access to a LightCache instance, managing the entry and eviction of items in the cache.
pub mod policy;
#[doc(inline)]
pub use policy::Policy;

/// A RefreshCache provides a simple interface for caching values with a time-to-live policy.
pub mod refresh;
#[doc(inline)]
pub use refresh::RefreshCache;

#[doc(hidden)] pub mod constants_for_benchmarking;