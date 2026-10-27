use crate::app::workers::ReaderJobs;
use std::thread::JoinHandle;

#[derive(Default)]
pub(super) struct RetiringJobs {
    threads: Vec<JoinHandle<()>>,
}

impl RetiringJobs {
    pub fn available(&mut self) -> bool {
        self.threads.retain(|thread| !thread.is_finished());
        self.threads.len() < 4
    }

    pub fn retire(&mut self, jobs: ReaderJobs) {
        jobs.stop();
        self.threads.push(std::thread::spawn(move || drop(jobs)));
    }
}

impl Drop for RetiringJobs {
    fn drop(&mut self) {
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
    }
}
