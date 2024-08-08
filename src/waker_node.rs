use std::task::Waker;

pub(crate) struct Wakers {
    // How many failures have been made to insert the value
    curr_try: usize,
    // wakers to notify when the value is inserted
    pub(crate) wakers: Vec<Waker>,
}

impl Wakers {
    pub(crate) fn start() -> Self {
        Wakers {
            curr_try: 0,
            wakers: Vec::new(),
        }
    }

    pub(crate) fn join(&mut self, waker: Waker) {
        self.wakers.push(waker);
    }

    // how many tries have been made to insert the value
    pub(crate) fn attempts(&mut self) -> &usize {
        &self.curr_try
    }

    #[inline]
    pub(crate) fn alert_all(self) {
        for waker in self.wakers {
            waker.wake();
        }
    }
}