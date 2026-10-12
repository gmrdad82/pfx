use std::path::Path;

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

pub struct Job;

impl Job {
    pub fn disabled() -> Job {
        Job
    }

    pub fn start(_tool: &str, _app: Option<&str>, _label: &str) -> Job {
        Job
    }

    pub fn plan(&mut self, _names: &[&'static str]) {}

    pub fn archiving(&mut self, _runs: bool) {}

    pub fn detail(&mut self, _detail: String) {}

    pub fn device(&mut self, _device: &str) {}

    pub fn out(&mut self, _dir: &Path) {}

    pub fn stage(&self, _name: &str, _total: Option<u64>, _unit: &str) {}

    pub fn note(&self, _note: &str) {}

    pub fn progress(&self, _done: u64) {}

    pub fn rendered(&self) {}

    pub fn pgpu(&self, _pid: Option<u32>) {}

    pub fn end<T>(self, result: Result<T, String>) -> Result<T, String> {
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
