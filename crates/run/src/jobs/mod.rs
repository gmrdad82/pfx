pub mod process;
pub mod progress;
pub mod state;
pub mod store;
pub mod supervise;

use std::path::Path;

pub use state::{Asked, Job, Next, State};

pub fn alive(dir: &Path, job: &Job) -> bool {
    supervise::held(dir)
        && job
            .supervisor
            .as_ref()
            .is_some_and(|me| me.start.is_none() || process::same(me))
}

pub fn seen(dir: &Path, job: &Job) -> State {
    if job.state.live() && !alive(dir, job) {
        State::Stopped
    } else {
        job.state
    }
}
