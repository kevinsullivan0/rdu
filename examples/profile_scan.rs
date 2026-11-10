use std::{
    alloc::{GlobalAlloc, Layout, System},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    time::Instant,
};
struct Allocator;
static TRACK: AtomicBool = AtomicBool::new(false);
static ALLOCS: AtomicU64 = AtomicU64::new(0);
static BYTES: AtomicU64 = AtomicU64::new(0);
static REALLOCS: AtomicU64 = AtomicU64::new(0);
static FREES: AtomicU64 = AtomicU64::new(0);
// SAFETY: every operation forwards the identical pointer/layout to System;
// counters never allocate, inspect or change application memory.
unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(l) };
        if TRACK.load(Ordering::Relaxed) {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
            BYTES.fetch_add(l.size() as u64, Ordering::Relaxed);
        }
        p
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        let p = unsafe { System.alloc_zeroed(l) };
        if TRACK.load(Ordering::Relaxed) {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
            BYTES.fetch_add(l.size() as u64, Ordering::Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        if TRACK.load(Ordering::Relaxed) {
            FREES.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        let p = unsafe { System.realloc(p, l, n) };
        if TRACK.load(Ordering::Relaxed) {
            REALLOCS.fetch_add(1, Ordering::Relaxed);
            BYTES.fetch_add(n as u64, Ordering::Relaxed);
        }
        p
    }
}
#[global_allocator]
static GLOBAL: Allocator = Allocator;
fn main() {
    let mut args = std::env::args_os().skip(1);
    let path = std::path::PathBuf::from(args.next().expect("PATH [REPEATS] [THREADS]"));
    let repeats = args
        .next()
        .and_then(|s| s.to_str().and_then(|s| s.parse().ok()))
        .unwrap_or(1usize);
    let threads = args
        .next()
        .and_then(|s| s.to_str().and_then(|s| s.parse().ok()))
        .unwrap_or(8usize);
    for _ in 0..repeats {
        rdu::profile::reset();
        for c in [&ALLOCS, &BYTES, &REALLOCS, &FREES] {
            c.store(0, Ordering::Relaxed);
        }
        TRACK.store(true, Ordering::Relaxed);
        let start = Instant::now();
        let tree = rdu::scan::scan(
            &path,
            &rdu::scan::Options {
                threads,
                ..Default::default()
            },
        );
        let elapsed = start.elapsed();
        TRACK.store(false, Ordering::Relaxed);
        let tree = tree.unwrap();
        let report = rdu::profile::report();
        let mut calls = [0u64; 15];
        let mut ns = [0u64; 15];
        for c in std::iter::once(&report.main).chain(report.workers.iter()) {
            for i in 0..15 {
                calls[i] += c.calls[i];
                ns[i] += c.nanoseconds[i];
            }
        }
        let events: serde_json::Map<String, serde_json::Value> = rdu::profile::EVENTS
            .iter()
            .enumerate()
            .map(|(i, name)| {
                (
                    name.to_string(),
                    serde_json::json!({"calls":calls[i],"nanoseconds":ns[i]}),
                )
            })
            .collect();
        println!(
            "{}",
            serde_json::json!({"diagnostic_only":true,"allocated":tree.dirs[0].allocated,"apparent":tree.dirs[0].apparent,"files":tree.files,"directories":tree.dirs.len(),"errors":tree.errors,"wall_seconds":elapsed.as_secs_f64(),"events":events,"allocations":{"calls":ALLOCS.load(Ordering::Relaxed),"bytes_requested":BYTES.load(Ordering::Relaxed),"reallocations":REALLOCS.load(Ordering::Relaxed),"deallocations":FREES.load(Ordering::Relaxed)}})
        );
    }
}
