use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

pub const TOOL: &str = "pfx";
pub const HD_DETAIL: &str = "hd: ";
pub const ASSETS_DETAIL: &str = "assets: ";

pub mod stages {
    pub const BUILD: &str = "build";
    pub const TRACE: &str = "trace";
    pub const TRACE_PROP: &str = "trace prop";
    pub const TRACE_REFLECTION: &str = "trace reflection";
    pub const DERIVE: &str = "derive";
    pub const MASTER: &str = "master";
    pub const ENCODE: &str = "encode";
    pub const ARCHIVE: &str = "archive";

    pub const APP_STILL: &[&str] = &[BUILD, TRACE, DERIVE, ARCHIVE];
    pub const APP_CLIP: &[&str] = &[BUILD, TRACE, MASTER, ENCODE, ARCHIVE];
    pub const APP_ENCODE: &[&str] = &[ENCODE, ARCHIVE];
    pub const ASSET_STILL: &[&str] = &[TRACE, ARCHIVE];
    pub const ASSET_CLIP: &[&str] = &[TRACE, MASTER, ENCODE, ARCHIVE];
    pub const ASSET_ENCODE: &[&str] = &[ENCODE, ARCHIVE];
    pub const ARCHIVE_RETRY: &[&str] = &[ARCHIVE];
}

pub struct Job {
    inner: Option<pfx_report::Job>,
    rendered: AtomicU64,
    plan: Vec<&'static str>,
}

impl Job {
    pub fn disabled() -> Job {
        Job {
            inner: None,
            rendered: AtomicU64::new(0),
            plan: Vec::new(),
        }
    }

    pub fn start(tool: &str, app: Option<&str>, label: &str) -> Job {
        Job {
            inner: Some(pfx_report::job(tool, app, label)),
            rendered: AtomicU64::new(0),
            plan: Vec::new(),
        }
    }

    pub fn plan(&mut self, names: &[&'static str]) {
        self.plan = names.to_vec();
        self.declare();
    }

    pub fn archiving(&mut self, runs: bool) {
        if runs {
            self.stage(stages::ARCHIVE, None, "outputs");
        } else if self.plan.contains(&stages::ARCHIVE) {
            self.plan.retain(|name| *name != stages::ARCHIVE);
            self.declare();
        }
    }

    fn declare(&self) {
        if let Some(job) = &self.inner {
            job.stages(&self.plan);
        }
    }

    fn with(&mut self, set: impl FnOnce(pfx_report::Job) -> pfx_report::Job) {
        self.inner = self.inner.take().map(set);
    }

    pub fn detail(&mut self, detail: String) {
        self.with(|job| job.detail(&detail));
    }

    pub fn device(&mut self, device: &str) {
        self.with(|job| job.device(device));
    }

    pub fn out(&mut self, dir: &Path) {
        self.with(|job| job.out(dir));
    }

    pub fn stage(&self, name: &str, total: Option<u64>, unit: &str) {
        self.rendered.store(0, Ordering::Relaxed);
        if let Some(job) = &self.inner {
            job.stage(name, total, unit);
        }
    }

    pub fn note(&self, note: &str) {
        if let Some(job) = &self.inner {
            job.stage_note(note);
        }
    }

    pub fn progress(&self, done: u64) {
        if let Some(job) = &self.inner {
            job.progress(done);
        }
    }

    pub fn rendered(&self) {
        let done = self.rendered.fetch_add(1, Ordering::Relaxed) + 1;
        self.progress(done);
    }

    pub fn pgpu(&self, pid: Option<u32>) {
        if let Some(job) = &self.inner {
            job.gpu_process(pid);
        }
    }

    pub fn end<T>(self, result: Result<T, String>) -> Result<T, String> {
        if let Some(job) = &self.inner {
            match &result {
                Ok(_) => job.finish(0),
                Err(e) => job.fail(1, e),
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::{ASSETS_DETAIL, HD_DETAIL, TOOL, stages};

    #[test]
    fn the_plans_use_only_the_stage_names_the_renderers_call() {
        let known = [
            stages::BUILD,
            stages::TRACE,
            stages::TRACE_PROP,
            stages::TRACE_REFLECTION,
            stages::DERIVE,
            stages::MASTER,
            stages::ENCODE,
            stages::ARCHIVE,
        ];
        for plan in [
            stages::APP_STILL,
            stages::APP_CLIP,
            stages::APP_ENCODE,
            stages::ASSET_STILL,
            stages::ASSET_CLIP,
            stages::ASSET_ENCODE,
            stages::ARCHIVE_RETRY,
        ] {
            assert!(plan.iter().all(|name| known.contains(name)));
            let mut seen = plan.to_vec();
            seen.dedup();
            assert_eq!(seen, plan);
        }
    }

    #[test]
    fn the_job_names_pfx_and_the_detail_prefixes_ptop_keys_on() {
        assert_eq!(TOOL, "pfx");
        assert_eq!(HD_DETAIL, "hd: ");
        assert_eq!(ASSETS_DETAIL, "assets: ");
    }
}
