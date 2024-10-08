use super::{max_parrellism, LightMap, Shard};

use hashbrown::raw::RawTable;
use hashbrown::HashMap;

use std::hash::BuildHasher;

use parking_lot::{Mutex, RwLock};

pub struct MapBuilder {
    pub(crate) shards: Option<usize>,
    pub(crate) estimated_size: Option<usize>,
}

impl MapBuilder {
    pub fn new() -> Self {
        MapBuilder {
            shards: None,
            estimated_size: None,
        }
    }

    /// Set the number of shards
    /// 
    /// This will be rounded up to the next power of two, but by default it will be 4 times the number of cores
    /// since this is the number of possible parrallel operations
    pub fn shards(mut self, shards: usize) -> Self {
        self.shards = Some(shards);
        self
    }

    /// Set the estimated size of the map
    /// 
    /// Used to preallocate the map
    pub fn estimated_size(mut self, estimated_size: usize) -> Self {
        self.estimated_size = Some(estimated_size);
        self
    }

    pub fn build<K, V, S: BuildHasher>(self, build_hasher: S) -> LightMap<K, V, S> {
        let shards = self
            .shards
            .unwrap_or_else(|| max_parrellism() * 4)
            .next_power_of_two();

        if let Some(estimated_size) = self.estimated_size {
            if estimated_size > shards {
                let per_shard = (estimated_size / shards) * 2;

                let shards = (0..shards)
                    .map(|_| Shard {
                        waiters: Mutex::new(HashMap::new()),
                        table: RwLock::new(RawTable::with_capacity(per_shard)),
                    })
                    .collect();

                return LightMap {
                    shards,
                    build_hasher,
                };
            }
        }

        // we have no estimated size or theres more shards
        let shards = (0..shards)
            .map(|_| Shard {
                waiters: Mutex::new(HashMap::new()),
                table: RwLock::new(RawTable::new()),
            })
            .collect();

        LightMap {
            shards,
            build_hasher,
        }
    }
}
