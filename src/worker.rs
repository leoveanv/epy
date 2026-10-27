use std::sync::{Condvar, Mutex};

use crate::cancellation::Cancellation;

pub(crate) struct WorkerRequest<T> {
    pub job: T,
    pub generation: u64,
    pub cancellation: Cancellation,
}

struct Pending<T, R> {
    request: Option<WorkerRequest<T>>,
    active: Option<Cancellation>,
    result: Option<R>,
    generation: u64,
    stopped: bool,
}

impl<T, R> Default for Pending<T, R> {
    fn default() -> Self {
        Self {
            request: None,
            active: None,
            result: None,
            generation: 0,
            stopped: false,
        }
    }
}

impl<T, R> Pending<T, R> {
    fn cancel(&mut self) {
        if let Some(active) = self.active.take() {
            active.cancel();
        }
        self.request = None;
        self.result = None;
        self.generation = self.generation.wrapping_add(1);
    }
}

pub(crate) struct ReplaceableMailbox<T, R> {
    pending: Mutex<Pending<T, R>>,
    wake: Condvar,
}

impl<T, R> Default for ReplaceableMailbox<T, R> {
    fn default() -> Self {
        Self {
            pending: Mutex::new(Pending::default()),
            wake: Condvar::new(),
        }
    }
}

impl<T, R> ReplaceableMailbox<T, R> {
    pub fn submit(&self, job: T) -> u64 {
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        pending.cancel();
        let cancellation = Cancellation::default();
        let generation = pending.generation;
        pending.active = Some(cancellation.clone());
        pending.request = Some(WorkerRequest {
            job,
            generation,
            cancellation,
        });
        self.wake.notify_one();
        generation
    }

    pub fn cancel(&self) {
        self.pending
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .cancel();
    }

    pub fn stop(&self) {
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        pending.cancel();
        pending.stopped = true;
        self.wake.notify_one();
    }

    pub fn receive(&self) -> Option<WorkerRequest<T>> {
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        loop {
            if pending.stopped {
                return None;
            }
            if let Some(request) = pending.request.take() {
                return Some(request);
            }
            pending = self
                .wake
                .wait(pending)
                .unwrap_or_else(|error| error.into_inner());
        }
    }

    pub fn publish(&self, generation: u64, result: R) {
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if !pending.stopped && pending.generation == generation {
            pending.result = Some(result);
        }
    }

    pub fn take_result(&self) -> Option<R> {
        self.pending
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .result
            .take()
    }
}
