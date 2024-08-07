use criterion::BenchmarkId;
use criterion::Criterion;
use criterion::{criterion_group, criterion_main};

use quick_cache::sync::Cache;

use tokio::runtime;

use std::sync::{OnceLock, Mutex};

use light_cache::constants_for_benchmarking::{INSERT_MANY, GET_MANY};

static CACHE: OnceLock<Mutex<Cache<usize, usize>>> = OnceLock::new();

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
        cache.get_or_insert_async(&i, async { Ok::<_, ()>(1) }).await.unwrap();
    }
}

fn clear_cache(new_size: usize) {
    let mut cache_ref = CACHE.get().unwrap().lock().unwrap();

    let _ = std::mem::replace(&mut *cache_ref, Cache::new(new_size));
}

fn bencher(c: &mut Criterion) {
    let rt = runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();

    CACHE.get_or_init(|| Mutex::new(Cache::new(INSERT_MANY)));

    c.bench_function("quick cache insert many", |b| b.to_async(&rt).iter(insert_many));

    clear_cache(GET_MANY);

    c.bench_function("quick cache insert and lookup", |b| b.to_async(&rt).iter(insert_and_lookup));

    clear_cache(GET_MANY);

    c.bench_function("quick cache get or insert many", |b| b.to_async(&rt).iter(get_or_insert_many));
}

criterion_group!(benches, bencher);
criterion_main!(benches);
