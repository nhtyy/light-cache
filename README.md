# LightCache

Built on top of [Hashbrowns](https://docs.rs/hashbrown/latest/hashbrown/raw/struct.RawTable.html#) RawTables | 
LightCache's straightfoward design makes implementing most eviction strategies a breeze.

## Description
LightCache is designed with Rust's async patterns at heart: any reads and write to LightCache (or the underlying map) are always non-blocking.
As well as providing a simple interface to work with it | LightCache is just as fast or faster as many other popular caching libraries.

Fundamentally | this is because we bound `V: Clone` | so even if there was contention across threads | it's just a few atomic operations between them.
Instead larger `V` should be wrapped in an `Arc` before being inserted into the cache.

LightCache currently ships with a couple predefined eviction policies and some helpers for creating new ones. 

## Benchmarks
According to [mokabench](https://github.com/moka-rs/mokabench) the base cache with no policy is the fastest available.

### S3 (MokaBench)
Performed on Apple MBP 2021 Pro M1

| Cache                        | Max Capacity | Clients | Inserts | Reads   | Hit Ratio | Duration Secs |
|------------------------------|--------------|---------|---------|---------|-----------|---------------|
| Mini Moka Unsync Cache        | 100000       | 1       | 14695344| 16407702| 10.436    | 3.887         |
| HashLink (LRU w/ Mutex)       | 100000       | 1       | 16025830| 16407702| 2.327     | 2.965         |
| HashLink (LRU w/ Mutex)       | 100000       | 3       | 16025819| 16407702| 2.327     | 4.897         |
| HashLink (LRU w/ Mutex)       | 100000       | 6       | 16025888| 16407702| 2.327     | 5.980         |
| QuickCache Sync Cache         | 100000       | 1       | 14300847| 16407702| 12.841    | 4.835         |
| QuickCache Sync Cache         | 100000       | 3       | 14301577| 16407702| 12.836    | 2.190         |
| QuickCache Sync Cache         | 100000       | 6       | 14301078| 16407702| 12.839    | 2.441         |
| LightCache Sync Cache         | 100000       | 1       | 1689882 | 16407702| 89.701    | 2.378         |
| LightCache Sync Cache         | 100000       | 3       | 1689882 | 16407702| 89.701    | 0.904         |
| LightCache Sync Cache         | 100000       | 6       | 1689884 | 16407702| 89.701    | 0.584         |
| LightCache Sync Cache LRU     | 100000       | 1       | 16025830| 16407702| 2.327     | 5.026         |
| LightCache Sync Cache LRU     | 100000       | 3       | 16025891| 16407702| 2.327     | 6.931         |
| LightCache Sync Cache LRU     | 100000       | 6       | 16025813| 16407702| 2.327     | 8.978         |

### conc-map-bench results (dashmap vs internal LightMap)

https://github.com/nhtyy/conc-map-bench

