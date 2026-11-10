//! Diagnostic counters for one active scan. Opt in with `--features profile`.
//! Instrumented timings are not comparable to ordinary release timings.
use std::time::Instant;
use std::{cell::RefCell, sync::Mutex};

pub const EVENTS: [&str; 15] = [
    "openat",
    "fstat",
    "fstatat",
    "getattrlistbulk",
    "readdir",
    "cache_hit",
    "cache_miss",
    "root_reopen",
    "fallback_component_open",
    "cache_lock",
    "work_lock",
    "result_lock",
    "condition_wait",
    "worker_total",
    "aggregation",
];
#[derive(Clone, Copy)]
#[repr(usize)]
pub enum Event {
    Open,
    StatFd,
    StatAt,
    Bulk,
    ReadDir,
    CacheHit,
    CacheMiss,
    RootReopen,
    FallbackOpen,
    CacheLock,
    WorkLock,
    ResultLock,
    Wait,
    Worker,
    Aggregation,
}
#[derive(Clone, Debug, Default)]
pub struct Counters {
    pub calls: [u64; 15],
    pub nanoseconds: [u64; 15],
}
#[derive(Debug, Default)]
pub struct Report {
    pub main: Counters,
    pub workers: Vec<Counters>,
}
thread_local! {static LOCAL:RefCell<Counters>=RefCell::new(Counters::default());}
static WORKERS: Mutex<Vec<Counters>> = Mutex::new(Vec::new());
pub fn reset() {
    let mut workers = WORKERS.lock().unwrap();
    workers.clear();
    workers.reserve(64);
    LOCAL.with(|c| *c.borrow_mut() = Counters::default());
}
pub fn count(event: Event) {
    LOCAL.with(|c| c.borrow_mut().calls[event as usize] += 1);
}
pub fn report() -> Report {
    Report {
        main: LOCAL.with(|c| c.borrow().clone()),
        workers: WORKERS.lock().unwrap().clone(),
    }
}
pub struct Timer {
    event: Event,
    start: Instant,
}
impl Timer {
    pub fn new(event: Event) -> Self {
        Self {
            event,
            start: Instant::now(),
        }
    }
}
impl Drop for Timer {
    fn drop(&mut self) {
        let ns = self.start.elapsed().as_nanos().min(u64::MAX as u128) as u64;
        LOCAL.with(|c| {
            let mut c = c.borrow_mut();
            c.calls[self.event as usize] += 1;
            c.nanoseconds[self.event as usize] += ns;
        });
    }
}
pub struct Worker {
    start: Instant,
}
impl Worker {
    pub fn start() -> Self {
        LOCAL.with(|c| *c.borrow_mut() = Counters::default());
        Self {
            start: Instant::now(),
        }
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        LOCAL.with(|c| {
            let mut c = c.borrow_mut();
            c.calls[Event::Worker as usize] += 1;
            c.nanoseconds[Event::Worker as usize] +=
                self.start.elapsed().as_nanos().min(u64::MAX as u128) as u64;
            WORKERS.lock().unwrap().push(c.clone());
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn records_thread_local_events_and_finished_workers() {
        // The diagnostic collector supports one active scan per process.
        // Other library tests scan concurrently, so exercise its exact counts
        // in a child test process rather than racing their worker records.
        const CHILD: &str = "RDU_PROFILE_COUNTER_TEST_CHILD";
        if std::env::var_os(CHILD).is_none() {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "profile::tests::records_thread_local_events_and_finished_workers",
                    "--test-threads=1",
                ])
                .env(CHILD, "1")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "isolated profile assertions failed:\n{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        reset();
        count(Event::CacheHit);
        {
            let _timer = Timer::new(Event::StatAt);
        }
        std::thread::spawn(|| {
            let _worker = Worker::start();
            count(Event::CacheMiss);
        })
        .join()
        .unwrap();
        let r = report();
        assert_eq!(r.main.calls[Event::CacheHit as usize], 1);
        assert_eq!(r.main.calls[Event::StatAt as usize], 1);
        assert_eq!(r.workers.len(), 1);
        assert_eq!(r.workers[0].calls[Event::CacheMiss as usize], 1);
        assert_eq!(r.workers[0].calls[Event::Worker as usize], 1);
    }
}
