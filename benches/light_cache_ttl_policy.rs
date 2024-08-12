use criterion::Criterion;
use criterion::{criterion_group, criterion_main};
use hashbrown::hash_map::DefaultHashBuilder;
use light_cache::policy::TtlPolicy;
use tokio::runtime;

use std::sync::{OnceLock, Mutex};
use std::time::Duration;

use light_cache::constants_for_benchmarking::{INSERT_MANY, GET_MANY};
use light_cache::LightCache;

static CACHE: OnceLock<Mutex<LightCache<usize, usize, DefaultHashBuilder, TtlPolicy<usize, usize>>>> = OnceLock::new();

// Here we have an async function to benchmark
async fn insert_many() {
    let cache = CACHE.get().unwrap().lock().unwrap();

    for i in 0..INSERT_MANY {
        cache.insert(i, i);
    }
}

async fn insert_and_lookup() {
    let cache = CACHE.get().unwrap().lock().unwrap();

    for i in 0..GET_MANY {
        cache.insert(i, i);
    }

    for i in 0..GET_MANY {
        cache.get(&i);
    }
}

async fn get_or_insert_many() {
    let cache = CACHE.get().unwrap().lock().unwrap();

    for i in 0..GET_MANY {
        cache.get_or_insert(i, || async { i }).await;
    }
}

async fn get_or_insert_many_spawn_tasks() {
    let cache = CACHE.get().unwrap().lock().unwrap();

    let handles = (0..GET_MANY).map(|i| {
        let cache = cache.clone();
        tokio::spawn(async move {
            cache.get_or_insert(i, || async { i }).await;
        })
    });

    for handle in handles {
        handle.await.unwrap();
    }
}

fn clear_cache(amount: usize) {
    let mut cache_ref = CACHE.get().unwrap().lock().unwrap();

    let _ = std::mem::replace(&mut *cache_ref, new_ttl_policy_cache(amount));
}

fn new_ttl_policy_cache(capacity: usize) -> LightCache<usize, usize, DefaultHashBuilder, TtlPolicy<usize, usize>> {
    LightCache::from_parts_with_capacity(TtlPolicy::new(Duration::from_secs(5)), DefaultHashBuilder::default(), capacity)
}


fn bencher(c: &mut Criterion) {
    let rt = runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();

    CACHE.get_or_init(|| Mutex::new(new_ttl_policy_cache(INSERT_MANY)));

    c.bench_function("light cache insert many", |b| b.to_async(&rt).iter(insert_many));

    clear_cache(GET_MANY);

    c.bench_function("light cache insert and lookup", |b| b.to_async(&rt).iter(insert_and_lookup));

    clear_cache(GET_MANY);

    c.bench_function("light cache get or insert many spawned tasks", |b| b.to_async(&rt).iter(get_or_insert_many_spawn_tasks));

    clear_cache(GET_MANY);

    c.bench_function("light cache get or insert many", |b| b.to_async(&rt).iter(get_or_insert_many));
}

criterion_group!(benches, bencher);
criterion_main!(benches);
