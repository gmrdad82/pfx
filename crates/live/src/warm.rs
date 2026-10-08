use crate::frame::{OpaqueKey, opaque_pipeline};
use std::collections::VecDeque;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Condvar, Mutex};

pub(crate) type Arrival = (OpaqueKey, Option<wgpu::RenderPipeline>);

pub(crate) struct Job {
    pub device: wgpu::Device,
    pub layout: wgpu::PipelineLayout,
    pub module: wgpu::ShaderModule,
    pub cache: Option<wgpu::PipelineCache>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Progress {
    pub made: usize,
    pub total: usize,
    pub first_made: usize,
    pub first_total: usize,
    pub failed: usize,
}

impl Progress {
    pub fn fraction(self) -> f32 {
        if self.total == 0 {
            1.0
        } else {
            self.made as f32 / self.total as f32
        }
    }
}

#[derive(Debug)]
struct Shared {
    progress: Mutex<Progress>,
    changed: Condvar,
}

#[derive(Clone, Debug)]
pub struct Warming {
    shared: Arc<Shared>,
}

impl Warming {
    pub fn progress(&self) -> Progress {
        *self.shared.progress.lock().expect("warm progress")
    }

    pub fn first_ready(&self) -> bool {
        let progress = self.progress();
        progress.first_made >= progress.first_total
    }

    pub fn done(&self) -> bool {
        let progress = self.progress();
        progress.made >= progress.total
    }

    pub fn wait(&self) -> Progress {
        let mut progress = self.shared.progress.lock().expect("warm progress");
        while progress.made < progress.total {
            progress = self.shared.changed.wait(progress).expect("warm progress");
        }
        *progress
    }

    fn finished(&self, first: bool, made: bool) {
        let mut progress = self.shared.progress.lock().expect("warm progress");
        progress.made += 1;
        if first {
            progress.first_made += 1;
        }
        if !made {
            progress.failed += 1;
        }
        drop(progress);
        self.shared.changed.notify_all();
    }
}

struct Promise {
    key: OpaqueKey,
    first: bool,
    sender: Sender<Arrival>,
    warming: Warming,
    kept: bool,
}

impl Promise {
    fn keep(mut self, made: wgpu::RenderPipeline) -> bool {
        self.kept = true;
        self.warming.finished(self.first, true);
        self.sender.send((self.key, Some(made))).is_ok()
    }
}

impl Drop for Promise {
    fn drop(&mut self) {
        if !self.kept {
            self.warming.finished(self.first, false);
            let _ = self.sender.send((self.key, None));
        }
    }
}

pub fn default_workers() -> usize {
    std::thread::available_parallelism()
        .map_or(1, |threads| threads.get())
        .saturating_sub(1)
        .max(1)
}

pub(crate) fn spawn(
    job: Job,
    keys: Vec<OpaqueKey>,
    first: usize,
    workers: usize,
    sender: Sender<Arrival>,
) -> Warming {
    let warming = Warming {
        shared: Arc::new(Shared {
            progress: Mutex::new(Progress {
                total: keys.len(),
                first_total: first,
                ..Progress::default()
            }),
            changed: Condvar::new(),
        }),
    };
    let count = workers.max(1).min(keys.len());
    let queue: Arc<Mutex<VecDeque<(usize, OpaqueKey)>>> =
        Arc::new(Mutex::new(keys.into_iter().enumerate().collect()));
    let job = Arc::new(job);
    let abandon = {
        let sender = sender.clone();
        let warming = warming.clone();
        move |queue: &Mutex<VecDeque<(usize, OpaqueKey)>>| {
            let left: Vec<(usize, OpaqueKey)> =
                queue.lock().expect("warm queue").drain(..).collect();
            for (index, key) in left {
                drop(Promise {
                    key,
                    first: index < first,
                    sender: sender.clone(),
                    warming: warming.clone(),
                    kept: false,
                });
            }
        }
    };
    let mut spawned = 0;
    for _ in 0..count {
        let queue = Arc::clone(&queue);
        let job = Arc::clone(&job);
        let sender = sender.clone();
        let warming = warming.clone();
        let abandon = abandon.clone();
        let worker = std::thread::Builder::new()
            .name("pito warm".into())
            .spawn(move || {
                loop {
                    let next = queue.lock().expect("warm queue").pop_front();
                    let Some((index, key)) = next else { break };
                    let promise = Promise {
                        key,
                        first: index < first,
                        sender: sender.clone(),
                        warming: warming.clone(),
                        kept: false,
                    };
                    let made = opaque_pipeline(
                        &job.device,
                        &job.layout,
                        &job.module,
                        job.cache.as_ref(),
                        key,
                    );
                    if !promise.keep(made) {
                        abandon(&queue);
                        break;
                    }
                }
            });
        if worker.is_err() {
            break;
        }
        spawned += 1;
    }
    if spawned == 0 {
        abandon(&queue);
    }
    warming
}
