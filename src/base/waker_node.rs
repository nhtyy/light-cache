use std::task::Waker;

pub(crate) struct WakerNode {
    curr_try: usize,
    active: bool,
    pub(crate) wakers: Vec<Waker>,
}

impl WakerNode {
    pub(crate) fn start() -> Self {
        WakerNode {
            curr_try: 0,
            active: true,
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

    pub(crate) fn is_active(&self) -> bool {
        self.active
    }

    pub(crate) fn halt(&mut self) -> Vec<Waker> {
        self.active = false;

        std::mem::take(&mut self.wakers)
    }

    pub(crate) fn activate(&mut self) -> usize {
        self.curr_try += 1;
        self.active = true;

        self.curr_try
    }
}