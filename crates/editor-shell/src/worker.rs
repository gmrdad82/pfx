use std::fmt;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, TryRecvError};

pub type Wake = Arc<dyn Fn() + Send + Sync>;

pub enum Poll<T> {
    Idle,
    Busy,
    Done(T),
    Lost,
}

struct Ring(Wake);

impl Drop for Ring {
    fn drop(&mut self) {
        (self.0)();
    }
}

pub struct Worker<T> {
    wake: Wake,
    pending: Option<Receiver<T>>,
}

impl<T> fmt::Debug for Worker<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Worker")
            .field("busy", &self.pending.is_some())
            .finish_non_exhaustive()
    }
}

impl<T: Send + 'static> Worker<T> {
    pub fn new(wake: Wake) -> Worker<T> {
        Worker {
            wake,
            pending: None,
        }
    }

    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }

    pub fn start(&mut self, job: impl FnOnce() -> T + Send + 'static) {
        let (send, receive) = mpsc::channel();
        let ring = Ring(self.wake.clone());
        std::thread::spawn(move || {
            let ring = ring;
            let send = send;
            let _ = send.send(job());
            drop(send);
            drop(ring);
        });
        self.pending = Some(receive);
    }

    pub fn poll(&mut self) -> Poll<T> {
        let Some(pending) = &self.pending else {
            return Poll::Idle;
        };
        match pending.try_recv() {
            Ok(value) => {
                self.pending = None;
                Poll::Done(value)
            }
            Err(TryRecvError::Empty) => Poll::Busy,
            Err(TryRecvError::Disconnected) => {
                self.pending = None;
                Poll::Lost
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn a_job_runs_off_the_calling_thread_and_wakes_it() {
        let (woke, wakes) = mpsc::channel();
        let mut worker = Worker::new(Arc::new(move || {
            let _ = woke.send(());
        }));
        assert!(matches!(worker.poll(), Poll::Idle));
        let (open, gate) = mpsc::channel::<()>();
        let caller = std::thread::current().id();
        worker.start(move || {
            let _ = gate.recv_timeout(Duration::from_secs(10));
            std::thread::current().id() != caller
        });
        assert!(worker.busy());
        assert!(matches!(worker.poll(), Poll::Busy));
        open.send(()).unwrap();
        wakes.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(matches!(worker.poll(), Poll::Done(true)));
        assert!(!worker.busy());
    }

    #[test]
    fn a_job_that_panics_is_lost_and_still_wakes() {
        let (woke, wakes) = mpsc::channel();
        let mut worker: Worker<u32> = Worker::new(Arc::new(move || {
            let _ = woke.send(());
        }));
        worker.start(|| panic!("the worker panicked on purpose"));
        wakes.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(matches!(worker.poll(), Poll::Lost));
        assert!(!worker.busy());
    }
}
