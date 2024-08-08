use criterion::Criterion;
use criterion::{criterion_group, criterion_main};

use quick_cache::sync::Cache;

use tokio::runtime;

use std::sync::Arc;
use std::sync::{Mutex, OnceLock};

use light_cache::constants_for_benchmarking::{GET_MANY, INSERT_MANY};

static CACHE: OnceLock<Mutex<Arc<Cache<usize, usize>>>> = OnceLock::new();

async fn get_or_insert_many_spawn_tasks() {
    let cache = CACHE.get().unwrap().lock().unwrap();

    let handles = (0..GET_MANY).map(|i| {
        let cache = cache.clone();
        tokio::spawn(async move {
            cache
                .get_or_insert_async(&i, async { Ok::<_, ()>(1) })
                .await
                .unwrap();
        })
    });

    for handle in handles {
        handle.await.unwrap();
    }
}

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

    let _ = std::mem::replace(&mut *cache_ref, Arc::new(Cache::new(new_size)));
}

fn bencher(c: &mut Criterion) {
    let rt = runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();

    CACHE.get_or_init(|| Mutex::new(Arc::new(Cache::new(INSERT_MANY))));

    c.bench_function("quick cache insert many", |b| {
        b.to_async(&rt).iter(insert_many)
    });

    clear_cache(GET_MANY);

    c.bench_function("quick cache insert and lookup", |b| {
        b.to_async(&rt).iter(insert_and_lookup)
    });

    clear_cache(GET_MANY);

    c.bench_function("quick cache get or insert many", |b| {
        b.to_async(&rt).iter(get_or_insert_many)
    });

    clear_cache(GET_MANY);

    c.bench_function("quick cache get or insert many spawn tasks", |b| {
        b.to_async(&rt).iter(get_or_insert_many_spawn_tasks)
    });
}

criterion_group!(benches, bencher);
criterion_main!(benches);
