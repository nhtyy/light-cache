use std::task::Waker;

pub(crate) struct Wakers {
    // in the event of a [`LightCache::get_or_insert_race`], this is the number of concurrent workers
    pub(crate) workers: usize,
    // wakers to notify when the value is inserted
    pub(crate) wakers: Vec<Waker>,
}

impl Wakers {
    pub(crate) fn start(waker: Waker) -> Self {
        Wakers {
            workers: 1,
            wakers: vec![waker]
        }
    }

    pub(crate) fn join_waiter(&mut self, waker: Waker) {
        self.wakers.push(waker);
    }

    pub(crate) fn join_worker(&mut self) {
        self.workers += 1;
    }

    /// Returns the number of workers that have not yet completed
    pub(crate) fn remove_worker(&mut self) -> usize {
        self.workers = self.workers.checked_sub(1).unwrap_or(0);

        self.workers
    }

    pub(crate) fn alert_all(&self) {
        for waker in &self.wakers {
            waker.wake_by_ref();
        }
    }

    #[inline]
    pub(crate) fn finish(self) {
        for waker in self.wakers {
            waker.wake();
        }
    }
}