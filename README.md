# LightCache

Built on top of [Hashbrowns](https://docs.rs/hashbrown/latest/hashbrown/raw/struct.RawTable.html#) RawTables, 
LightCache's straightfoward design makes implementing most eviction strategies a breeze.

## Description
LightCache is designed with Rust's async patterns at heart: any reads and write to LightCache (or the underlying map) are always non-blocking.
As well as providing a simple interface to work with it, LightCache is just as fast or faster as many other popular caching libraries.

Fundamentally, this is because we bound `V: Clone`, so even if there was contention across threads, it's just a few atomic operations between them.
Instead larger `V` should be wrapped in an `Arc` before being inserted into the cache.

LightCache currently ships with a couple predefined eviction policies and some helpers for creating new ones. 

### Plans
- Proper benchmarks
- Create a `KeyOrHash<K>` to manage keys longer than 32 bytes
- LRU
- LFU
- get by ref
