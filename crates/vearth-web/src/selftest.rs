//! Browser checks for the platform layer, run from `web/selftest.html`.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use vearth_platform::rt;
use vearth_platform::thread;
use vearth_platform::time::Instant;
use wasm_bindgen::prelude::*;

fn check(name: &str, ok: bool, detail: String, report: &mut Vec<String>) {
    let line = format!("{} {name}: {detail}", if ok { "PASS" } else { "FAIL" });
    log::info!("{line}");
    report.push(line);
}

/// Exercises threads, the reactor, timers, the blocking pool and panic
/// recovery. Returns one line per check.
#[wasm_bindgen]
pub fn run_platform_selftest() -> String {
    let mut report = Vec::new();

    let parallelism = thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(0);
    check(
        "available_parallelism",
        parallelism > 0,
        format!("{parallelism}"),
        &mut report,
    );

    let counter = Arc::new(AtomicUsize::new(0));
    let started = Instant::now();
    let handles: Vec<_> = (0..4)
        .map(|index| {
            let counter = counter.clone();
            thread::Builder::new()
                .name(format!("selftest-{index}"))
                .spawn(move || {
                    thread::sleep(Duration::from_millis(50));
                    counter.fetch_add(1, Ordering::SeqCst);
                    index * 10
                })
                .expect("spawn")
        })
        .collect();
    let values: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    check(
        "thread spawn/join",
        values == [0, 10, 20, 30] && counter.load(Ordering::SeqCst) == 4,
        format!("{values:?} in {:?}", started.elapsed()),
        &mut report,
    );

    let panicked = thread::spawn(|| -> u32 { panic!("expected selftest panic") }).join();
    check(
        "thread panic is caught",
        panicked.is_err(),
        format!("{}", panicked.is_err()),
        &mut report,
    );

    let caught = std::panic::catch_unwind(|| {
        let v: Vec<u8> = Vec::new();
        v[3]
    });
    check(
        "catch_unwind",
        caught.is_err(),
        "index panic recovered".into(),
        &mut report,
    );

    let runtime = rt::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let started = Instant::now();
    let slept = runtime.block_on(async {
        rt::time::sleep(Duration::from_millis(120)).await;
        Instant::now()
    });
    let elapsed = slept - started;
    check(
        "reactor sleep",
        elapsed >= Duration::from_millis(115) && elapsed < Duration::from_millis(600),
        format!("{elapsed:?}"),
        &mut report,
    );

    let timed_out = runtime.block_on(rt::time::timeout(
        Duration::from_millis(30),
        rt::time::sleep(Duration::from_secs(5)),
    ));
    check(
        "timeout",
        timed_out.is_err(),
        format!("{timed_out:?}"),
        &mut report,
    );

    let joined = runtime.block_on(async {
        let tasks: Vec<_> = (0..16u64)
            .map(|i| {
                rt::spawn(async move {
                    rt::time::sleep(Duration::from_millis(10 * (i % 4))).await;
                    let square = rt::task::spawn_blocking(move || i * i).await.unwrap();
                    square + 1
                })
            })
            .collect();
        let mut sum = 0;
        for task in tasks {
            sum += task.await.unwrap();
        }
        sum
    });
    let expected: u64 = (0..16u64).map(|i| i * i + 1).sum();
    check(
        "spawn + spawn_blocking",
        joined == expected,
        format!("{joined} == {expected}"),
        &mut report,
    );

    let aborted = runtime.block_on(async {
        let task = rt::spawn(async {
            rt::time::sleep(Duration::from_secs(10)).await;
        });
        task.abort();
        task.await
    });
    check(
        "abort",
        aborted.as_ref().is_err_and(|e| e.is_cancelled()),
        format!("{aborted:?}"),
        &mut report,
    );

    let task_panic = runtime.block_on(rt::spawn(async { panic!("expected task panic") }));
    check(
        "task panic",
        task_panic.as_ref().is_err_and(|e| e.is_panic()),
        format!("{:?}", task_panic.err()),
        &mut report,
    );

    let fetched = runtime.block_on(rt::spawn_local_on_reactor(|| async {
        let global: web_sys_global::Global = js_sys::global().unchecked_into();
        let response = wasm_bindgen_futures::JsFuture::from(global.fetch("thread-worker.js"))
            .await
            .map_err(|e| format!("{e:?}"))?;
        let status = js_sys::Reflect::get(&response, &"status".into())
            .ok()
            .and_then(|s| s.as_f64())
            .unwrap_or(0.0);
        Ok::<_, String>(status)
    }));
    check(
        "reactor fetch",
        matches!(fetched, Ok(Ok(200.0))),
        format!("{fetched:?}"),
        &mut report,
    );

    for threads in [1, 2, 4, 8] {
        let (allocating, computing) = scaling(threads);
        check(
            &format!("scaling x{threads}"),
            true,
            format!("alloc {allocating:.0} Mop/s, compute {computing:.0} Mop/s"),
            &mut report,
        );
    }

    report.join("\n")
}

/// Aggregate throughput of an allocation-heavy and an allocation-free loop
/// on `threads` workers, in millions of operations per second. Timing starts
/// once every worker is running.
fn scaling(threads: usize) -> (f64, f64) {
    const ITERATIONS: usize = 2_000_000;
    let run = |allocate: bool| {
        let barrier = Arc::new(std::sync::Barrier::new(threads + 1));
        let handles: Vec<_> = (0..threads)
            .map(|_| {
                let barrier = barrier.clone();
                thread::spawn(move || {
                    barrier.wait();
                    let mut sum = 0u64;
                    for i in 0..ITERATIONS {
                        if allocate {
                            let v: Vec<u64> = Vec::with_capacity(8 + i % 24);
                            sum = sum.wrapping_add(std::hint::black_box(v).capacity() as u64);
                        } else {
                            sum = sum.wrapping_mul(6364136223846793005).wrapping_add(i as u64);
                        }
                    }
                    sum
                })
            })
            .collect();
        barrier.wait();
        let started = Instant::now();
        for handle in handles {
            std::hint::black_box(handle.join().unwrap());
        }
        (threads * ITERATIONS) as f64 / started.elapsed().as_secs_f64() / 1e6
    };
    (run(true), run(false))
}

mod web_sys_global {
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen]
    extern "C" {
        pub type Global;
        #[wasm_bindgen(method)]
        pub fn fetch(this: &Global, url: &str) -> js_sys::Promise;
    }
}

use wasm_bindgen::JsCast;
