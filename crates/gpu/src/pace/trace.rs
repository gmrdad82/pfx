use std::io::Write;
use std::panic::Location;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use super::{Frozen, Moment, Timing};

pub const TRACE_VAR: &str = "PFX_TURN_TRACE";

pub const COLUMNS: &str =
    "at_ms\tkind\tsite\twork_ms\tlongest_ms\tgap_ms\tslices\tgpu\twall\tgiven\tuntimed\ttest";

static EPOCH: OnceLock<Instant> = OnceLock::new();
static LAST: Mutex<Option<Moment>> = Mutex::new(None);
static NEXT_SLICE: AtomicU8 = AtomicU8::new(0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Gpu,
    Wall,
    Given,
    Untimed,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Sources {
    pub gpu: u32,
    pub wall: u32,
    pub given: u32,
    pub untimed: u32,
}

impl Sources {
    pub fn add(&mut self, source: Source) {
        match source {
            Source::Gpu => self.gpu += 1,
            Source::Wall => self.wall += 1,
            Source::Given => self.given += 1,
            Source::Untimed => self.untimed += 1,
        }
    }

    pub fn slices(&self) -> u32 {
        self.gpu + self.wall + self.given + self.untimed
    }
}

fn path() -> Option<PathBuf> {
    std::env::var_os(TRACE_VAR)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
}

pub(super) fn arm() {
    if path().is_none() {
        return;
    }
    EPOCH.get_or_init(Instant::now);
    let mut last = LAST.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if last.is_none() {
        *last = Some(Moment::now(&Frozen::from_env()));
    }
}

pub(super) fn turned() {
    if path().is_none() {
        return;
    }
    let mut last = LAST.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    *last = Some(Moment::now(&Frozen::from_env()));
}

pub(super) fn created(site: &Location<'_>) {
    record("new", site, 0.0, 0.0, Sources::default());
}

pub(super) fn slice_timed(timing: Timing) {
    NEXT_SLICE.store(
        match timing {
            Timing::Gpu => 1,
            Timing::Cpu => 2,
        },
        Ordering::Relaxed,
    );
}

pub(super) fn take_slice() -> Option<Timing> {
    match NEXT_SLICE.swap(0, Ordering::Relaxed) {
        1 => Some(Timing::Gpu),
        2 => Some(Timing::Cpu),
        _ => None,
    }
}

pub(super) fn record(
    kind: &str,
    site: &Location<'_>,
    work_ms: f64,
    longest_ms: f64,
    sources: Sources,
) {
    let Some(path) = path() else {
        return;
    };
    record_to(&path, kind, site, work_ms, longest_ms, sources);
}

fn record_to(
    path: &std::path::Path,
    kind: &str,
    site: &Location<'_>,
    work_ms: f64,
    longest_ms: f64,
    sources: Sources,
) {
    let epoch = *EPOCH.get_or_init(Instant::now);
    let now = Moment::now(&Frozen::from_env());
    let gap_ms = {
        let last = LAST.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        last.map_or(0.0, |earlier| now.ms_since(earlier))
    };
    let at_ms = now.at.saturating_duration_since(epoch).as_secs_f64() * 1000.0;
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path);
    let Ok(mut file) = file else {
        return;
    };
    let line = format!(
        "{at_ms:.1}\t{kind}\t{}:{}\t{work_ms:.3}\t{longest_ms:.3}\t{gap_ms:.1}\t{}\t{}\t{}\t{}\t{}\t{}\n",
        site.file(),
        site.line(),
        sources.slices(),
        sources.gpu,
        sources.wall,
        sources.given,
        sources.untimed,
        std::thread::current().name().unwrap_or("-"),
    );
    let _ = file.write_all(line.as_bytes());
}

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub at_ms: f64,
    pub kind: String,
    pub site: String,
    pub work_ms: f64,
    pub longest_ms: f64,
    pub gap_ms: f64,
    pub slices: u32,
    pub sources: Sources,
    pub test: String,
}

pub fn parse(text: &str) -> Vec<Row> {
    text.lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split('\t').collect();
            if fields.len() != 12 {
                return None;
            }
            let number = |index: usize| fields[index].parse::<f64>().ok();
            let count = |index: usize| fields[index].parse::<u32>().ok();
            Some(Row {
                at_ms: number(0)?,
                kind: fields[1].to_owned(),
                site: fields[2].to_owned(),
                work_ms: number(3)?,
                longest_ms: number(4)?,
                gap_ms: number(5)?,
                slices: count(6)?,
                sources: Sources {
                    gpu: count(7)?,
                    wall: count(8)?,
                    given: count(9)?,
                    untimed: count(10)?,
                },
                test: fields[11].to_owned(),
            })
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq)]
pub struct Finding {
    pub site: String,
    pub test: String,
    pub longest_ms: f64,
    pub gap_ms: f64,
}

pub fn findings(rows: &[Row], slice_ms: f64, gap_ms: f64) -> Vec<Finding> {
    rows.iter()
        .filter(|row| row.longest_ms > slice_ms || row.gap_ms > gap_ms)
        .map(|row| Finding {
            site: row.site.clone(),
            test: row.test.clone(),
            longest_ms: row.longest_ms,
            gap_ms: row.gap_ms,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: &str = "0.0\tturns\ta.rs:1\t12.000\t4.000\t0.0\t3\t3\t0\t0\t0\tone\n\
        90.0\tturns\tb.rs:2\t600.000\t590.000\t90.0\t1\t0\t1\t0\t0\ttwo\n\
        900.0\tpace\tc.rs:3\t1.000\t1.000\t810.0\t1\t1\t0\t0\t0\tthree\n\
        not a row\n";

    #[test]
    fn rows_parse_and_junk_is_skipped() {
        let rows = parse(TEXT);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[1].site, "b.rs:2");
        assert_eq!(rows[1].sources.wall, 1);
        assert_eq!(rows[2].test, "three");
        assert_eq!(rows[0].sources.slices(), 3);
    }

    #[test]
    fn findings_name_long_slices_and_long_gaps() {
        let found = findings(&parse(TEXT), 300.0, 500.0);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].site, "b.rs:2");
        assert_eq!(found[1].site, "c.rs:3");
        assert!(findings(&parse(TEXT), 1000.0, 1000.0).is_empty());
    }

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tmp/turn-trace-tests");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        let _ = std::fs::remove_file(&path);
        path
    }

    #[test]
    fn a_recorded_turn_reads_back_with_its_site() {
        let path = scratch("record.tsv");
        let mut sources = Sources::default();
        sources.add(Source::Gpu);
        sources.add(Source::Wall);
        record_to(&path, "turns", Location::caller(), 7.5, 4.25, sources);
        let rows = parse(&std::fs::read_to_string(&path).unwrap());
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].kind, "turns");
        assert!(rows[0].site.contains("trace.rs:"));
        assert_eq!((rows[0].work_ms, rows[0].longest_ms), (7.5, 4.25));
        assert_eq!((rows[0].slices, rows[0].sources), (2, sources));
    }

    #[test]
    fn a_paced_add_is_classified_by_the_pacers_timing() {
        let mut turns = super::super::Turns::new(1e9, false).with_wall_ms(1e9);
        slice_timed(Timing::Gpu);
        turns.add(1.0);
        slice_timed(Timing::Cpu);
        turns.add(1.0);
        turns.add(1.0);
        turns.add(f64::NAN);
        slice_timed(Timing::Gpu);
        take_slice();
        turns.add(1.0);
        assert_eq!(
            turns.sources,
            Sources {
                gpu: 1,
                wall: 1,
                given: 2,
                untimed: 1
            }
        );
    }

    #[test]
    fn a_poll_turns_only_once_the_wall_time_is_up() {
        let mut turns = super::super::Turns::new(1e9, false).with_wall_ms(1e9);
        assert!(!turns.poll());
        assert_eq!(turns.taken(), 0);
        let mut turns = super::super::Turns::new(1e9, false).with_wall_ms(1.0);
        std::thread::sleep(std::time::Duration::from_millis(5));
        assert!(turns.poll());
        assert_eq!(turns.taken(), 1);
    }

    #[test]
    fn sources_count_by_kind() {
        let mut sources = Sources::default();
        sources.add(Source::Gpu);
        sources.add(Source::Gpu);
        sources.add(Source::Untimed);
        assert_eq!((sources.gpu, sources.untimed, sources.slices()), (2, 1, 3));
    }

    const CHILD_VAR: &str = "PFX_TURN_TRACE_CHILD";

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn a_paced_gpu_run_traces_no_long_slice_and_no_long_gap() {
        if std::env::var_os(CHILD_VAR).is_some() {
            traced_workload();
            return;
        }
        let path = scratch("paced_run.tsv");
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                &format!(
                    "{}::a_paced_gpu_run_traces_no_long_slice_and_no_long_gap",
                    module_path!().trim_start_matches("pfx_gpu::")
                ),
                "--exact",
                "--ignored",
                "--test-threads=1",
            ])
            .env(TRACE_VAR, &path)
            .env(CHILD_VAR, "1")
            .status()
            .unwrap();
        assert!(status.success());
        assert!(path.exists(), "the child ran no test");
        let rows = parse(&std::fs::read_to_string(&path).unwrap());
        assert!(rows.len() >= 2, "the run traced {} turns", rows.len());
        assert!(
            rows.iter()
                .any(|row| row.sources.gpu + row.sources.wall > 0)
        );
        let found = findings(&rows, 300.0, 500.0);
        assert!(found.is_empty(), "{found:#?}");
    }

    fn traced_workload() {
        use super::super::{Pacer, Slice, Turns, Work};
        const GROUPS: u32 = 600;
        let gpu = pollster::block_on(crate::Gpu::headless()).unwrap();
        let device = &gpu.device;
        let mut turns = Turns::default();
        let shader = device.create_shader_module(crate::wgpu::ShaderModuleDescriptor {
            label: Some("trace test"),
            source: crate::wgpu::ShaderSource::Wgsl(
                "@group(0) @binding(0) var<storage, read_write> out: array<u32>;
                 @compute @workgroup_size(64)
                 fn main(@builtin(global_invocation_id) id: vec3<u32>) {
                     var a = id.x;
                     for (var i = 0u; i < 60000u; i = i + 1u) {
                         a = a * 1664525u + 1013904223u;
                     }
                     out[id.x] = a;
                 }"
                .into(),
            ),
        });
        let pipeline = device.create_compute_pipeline(&crate::wgpu::ComputePipelineDescriptor {
            label: Some("trace test"),
            layout: None,
            module: &shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let out = device.create_buffer(&crate::wgpu::BufferDescriptor {
            label: Some("out"),
            size: u64::from(GROUPS) * 64 * 4,
            usage: crate::wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let group = device.create_bind_group(&crate::wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[crate::wgpu::BindGroupEntry {
                binding: 0,
                resource: out.as_entire_binding(),
            }],
        });
        for _ in 0..3 {
            let mut pacer = Pacer::default();
            pacer
                .run(
                    "trace test",
                    device,
                    &gpu.queue,
                    Work::Dispatches { count: GROUPS },
                    |encoder, slice| {
                        let Slice::Dispatches { count, .. } = slice else {
                            unreachable!()
                        };
                        let mut pass = encoder.begin_compute_pass(&Default::default());
                        pass.set_pipeline(&pipeline);
                        pass.set_bind_group(0, &group, &[]);
                        pass.dispatch_workgroups(count, 1, 1);
                    },
                    |ms| turns.add(ms),
                )
                .unwrap();
            turns.time(|| {
                let mut encoder = device.create_command_encoder(&Default::default());
                {
                    let mut pass = encoder.begin_compute_pass(&Default::default());
                    pass.set_pipeline(&pipeline);
                    pass.set_bind_group(0, &group, &[]);
                    pass.dispatch_workgroups(16, 1, 1);
                }
                let index = gpu.queue.submit(Some(encoder.finish()));
                let _ = device.poll(crate::wgpu::PollType::Wait {
                    submission_index: Some(index),
                    timeout: None,
                });
            });
        }
        turns.turn();
    }
}
