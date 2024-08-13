//! A simple, fast and concurrent cache designed for async rust.
//! 
//! # Quick Start
//! The easiest way to get started with LightCache is to use [`crate::RefreshCache`]

pub mod cache;
#[doc(inline)]
pub use cache::LightCache;

/// The underlying map used by LightCache. Desinged to be fast and non-blocking for connurrent r/w.
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