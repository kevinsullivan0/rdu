//! Descriptor-free owner-local scheduling. No filesystem capability enters a queue.
use std::{
    collections::VecDeque,
    sync::{
        Condvar, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

struct Parking {
    generation: usize,
}
pub(super) struct Scheduler<T> {
    lanes: Vec<Mutex<VecDeque<T>>>,
    live: AtomicUsize,
    parking: Mutex<Parking>,
    ready: Condvar,
}
impl<T> Scheduler<T> {
    pub(super) fn new(workers: usize, initial: T) -> Self {
        let mut lanes: Vec<_> = (0..workers).map(|_| Mutex::new(VecDeque::new())).collect();
        lanes[0].get_mut().unwrap().push_back(initial);
        Self {
            lanes,
            live: AtomicUsize::new(1),
            parking: Mutex::new(Parking { generation: 0 }),
            ready: Condvar::new(),
        }
    }
    pub(super) fn register(&self, count: usize) {
        self.live.fetch_add(count, Ordering::Release);
    }
    fn transfer_limit(&self) -> usize {
        16usize.max(self.lanes.len() * 4)
    }
    fn claim(&self, worker: usize, local: &mut VecDeque<T>) -> Option<T> {
        for offset in 0..self.lanes.len() {
            let victim = (worker + offset) % self.lanes.len();
            #[cfg(feature = "profile")]
            let _timer = crate::profile::Timer::new(crate::profile::Event::WorkLock);
            let mut lane = self.lanes[victim].lock().unwrap();
            for _ in 0..4 {
                let item = if offset == 0 {
                    lane.pop_back()
                } else {
                    lane.pop_front()
                };
                let Some(item) = item else {
                    break;
                };
                local.push_back(item);
            }
            if let Some(item) = local.pop_back() {
                return Some(item);
            }
        }
        None
    }
    pub(super) fn acquire(&self, worker: usize, local: &mut VecDeque<T>) -> Option<T> {
        self.acquire_observed(worker, local, &mut |_| {})
    }
    fn acquire_observed(
        &self,
        worker: usize,
        local: &mut VecDeque<T>,
        observe: &mut impl FnMut(&str),
    ) -> Option<T> {
        if let Some(item) = local.pop_back() {
            return Some(item);
        }
        loop {
            if let Some(item) = self.claim(worker, local) {
                return Some(item);
            }
            observe("empty");
            let mut parking = self.parking.lock().unwrap();
            // Publishers drop lane guards before taking parking. Rechecking under this
            // guard closes empty-probe/publication/park races without lock inversion.
            if let Some(item) = self.claim(worker, local) {
                return Some(item);
            }
            if self.live.load(Ordering::Acquire) == 0 {
                return None;
            }
            let generation = parking.generation;
            while generation == parking.generation && self.live.load(Ordering::Acquire) != 0 {
                observe("parking");
                #[cfg(feature = "profile")]
                let _timer = crate::profile::Timer::new(crate::profile::Event::Wait);
                parking = self.ready.wait(parking).unwrap();
            }
        }
    }
    pub(super) fn finish(&self, worker: usize, local: &mut VecDeque<T>, children: Vec<T>) {
        local.extend(children);
        let mut published = 0;
        if local.len() > 1 {
            let limit = self.transfer_limit();
            let mut lane = self.lanes[worker].lock().unwrap();
            let capacity = (limit * 4).saturating_sub(lane.len());
            let count = (local.len() / 2).min(limit).min(capacity);
            for _ in 0..count {
                lane.push_back(local.pop_front().unwrap());
            }
            published = count;
        }
        let final_zero = self.live.fetch_sub(1, Ordering::AcqRel) == 1;
        if published != 0 || final_zero {
            let mut parking = self.parking.lock().unwrap();
            parking.generation = parking.generation.wrapping_add(1);
            self.ready.notify_all();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_registered_work_has_one_completion_and_private_work_is_live() {
        let scheduler = Scheduler::new(2, 0usize);
        let mut local = VecDeque::new();
        assert_eq!(scheduler.acquire(0, &mut local), Some(0));
        scheduler.register(2);
        scheduler.finish(0, &mut local, vec![1, 2]);
        assert_eq!(scheduler.live.load(Ordering::Acquire), 2);
        let mut peer = VecDeque::new();
        let stolen = scheduler.acquire(1, &mut peer).unwrap();
        assert!(stolen == 1 || stolen == 2);
        scheduler.finish(1, &mut peer, vec![]);
        assert_eq!(scheduler.live.load(Ordering::Acquire), 1);
        assert!(scheduler.acquire(0, &mut local).is_some());
        scheduler.finish(0, &mut local, vec![]);
        assert_eq!(scheduler.live.load(Ordering::Acquire), 0);
        assert!(scheduler.acquire(0, &mut local).is_none());
    }
    #[test]
    fn one_child_chain_stays_private_without_publication_wakes() {
        let scheduler = Scheduler::new(8, 0usize);
        let mut local = VecDeque::new();
        for i in 0..64 {
            assert_eq!(scheduler.acquire(0, &mut local), Some(i));
            let children = if i == 63 { vec![] } else { vec![i + 1] };
            scheduler.register(children.len());
            scheduler.finish(0, &mut local, children);
        }
        assert_eq!(
            scheduler.parking.lock().unwrap().generation,
            1,
            "only final zero needs a wake"
        );
    }
    #[test]
    fn publication_between_failed_steal_and_park_is_rechecked() {
        let scheduler = std::sync::Arc::new(Scheduler::new(2, 0usize));
        let mut local = VecDeque::new();
        assert_eq!(scheduler.acquire(0, &mut local), Some(0));
        let (empty_tx, empty_rx) = std::sync::mpsc::channel();
        let (resume_tx, resume_rx) = std::sync::mpsc::channel();
        let peer = scheduler.clone();
        let thread = std::thread::spawn(move || {
            let mut once = true;
            let mut local = VecDeque::new();
            let item = peer
                .acquire_observed(1, &mut local, &mut |event| {
                    if event == "empty" && once {
                        once = false;
                        empty_tx.send(()).unwrap();
                        resume_rx.recv().unwrap();
                    }
                })
                .unwrap();
            peer.finish(1, &mut local, vec![]);
            item
        });
        empty_rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        scheduler.register(2);
        scheduler.finish(0, &mut local, vec![1, 2]);
        resume_tx.send(()).unwrap();
        assert_eq!(thread.join().unwrap(), 1);
        assert_eq!(scheduler.acquire(0, &mut local), Some(2));
        scheduler.finish(0, &mut local, vec![]);
    }
    #[test]
    fn final_zero_wakes_an_already_registered_waiter() {
        let scheduler = std::sync::Arc::new(Scheduler::new(2, 0usize));
        let mut local = VecDeque::new();
        assert_eq!(scheduler.acquire(0, &mut local), Some(0));
        let (park_tx, park_rx) = std::sync::mpsc::channel();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let peer = scheduler.clone();
        let thread = std::thread::spawn(move || {
            let mut once = true;
            let item = peer.acquire_observed(1, &mut VecDeque::new(), &mut |event| {
                if event == "parking" && once {
                    once = false;
                    park_tx.send(()).unwrap();
                }
            });
            done_tx.send(item).unwrap();
        });
        park_rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        scheduler.finish(0, &mut local, vec![]);
        let result = done_rx.recv_timeout(std::time::Duration::from_secs(2));
        if result.is_err() {
            scheduler.ready.notify_all();
        }
        thread.join().unwrap();
        assert_eq!(result.unwrap(), None);
    }
    #[test]
    fn concurrent_registered_tree_is_completed_once_including_private_jobs() {
        for workers in [1, 8, 64] {
            let scheduler = Scheduler::new(workers, 0usize);
            let seen: Vec<_> = (0..127).map(|_| AtomicUsize::new(0)).collect();
            std::thread::scope(|scope| {
                for worker in 0..workers {
                    let scheduler = &scheduler;
                    let seen = &seen;
                    scope.spawn(move || {
                        let mut local = VecDeque::new();
                        while let Some(id) = scheduler.acquire(worker, &mut local) {
                            assert_eq!(seen[id].fetch_add(1, Ordering::Relaxed), 0);
                            let children = if id < 63 {
                                vec![id * 2 + 1, id * 2 + 2]
                            } else {
                                vec![]
                            };
                            scheduler.register(children.len());
                            scheduler.finish(worker, &mut local, children);
                        }
                    });
                }
            });
            assert_eq!(scheduler.live.load(Ordering::Acquire), 0);
            assert!(seen.iter().all(|n| n.load(Ordering::Relaxed) == 1));
        }
    }
    #[test]
    fn shared_lanes_remain_bounded_without_blocking_private_producer() {
        let scheduler = Scheduler::new(8, 0usize);
        let mut local = VecDeque::new();
        assert_eq!(scheduler.acquire(0, &mut local), Some(0));
        scheduler.register(10_000);
        scheduler.finish(0, &mut local, (1..=10_000).collect());
        assert!(local.len() > 9_000);
        assert!(scheduler.lanes[0].lock().unwrap().len() <= scheduler.transfer_limit() * 4);
        for _ in 0..100 {
            let _ = scheduler.acquire(0, &mut local).unwrap();
            scheduler.finish(0, &mut local, vec![]);
            assert!(scheduler.lanes[0].lock().unwrap().len() <= scheduler.transfer_limit() * 4);
        }
    }
}
