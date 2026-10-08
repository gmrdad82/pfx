use wasmtime::{ResourceLimiter, Result, StoreLimits, StoreLimitsBuilder};

use crate::error::Limit;
use crate::limits::{Limits, LogLimits};

pub struct Limiter {
    inner: StoreLimits,
    pub(crate) hit: Option<Limit>,
}

impl Limiter {
    fn new(limits: &Limits) -> Self {
        Self {
            inner: StoreLimitsBuilder::new()
                .memory_size(limits.memory_bytes)
                .table_elements(limits.table_elements)
                .instances(limits.instances)
                .tables(limits.tables)
                .memories(limits.memories)
                .trap_on_grow_failure(true)
                .build(),
            hit: None,
        }
    }
}

impl ResourceLimiter for Limiter {
    fn memory_growing(
        &mut self,
        current: usize,
        desired: usize,
        maximum: Option<usize>,
    ) -> Result<bool> {
        let grown = self.inner.memory_growing(current, desired, maximum);
        if !matches!(grown, Ok(true)) {
            self.hit = Some(Limit::Memory { bytes: desired });
        }
        grown
    }

    fn memory_grow_failed(&mut self, error: wasmtime::Error) -> Result<()> {
        self.inner.memory_grow_failed(error)
    }

    fn table_growing(
        &mut self,
        current: usize,
        desired: usize,
        maximum: Option<usize>,
    ) -> Result<bool> {
        let grown = self.inner.table_growing(current, desired, maximum);
        if !matches!(grown, Ok(true)) {
            self.hit = Some(Limit::Table { elements: desired });
        }
        grown
    }

    fn table_grow_failed(&mut self, error: wasmtime::Error) -> Result<()> {
        self.inner.table_grow_failed(error)
    }

    fn instances(&self) -> usize {
        self.inner.instances()
    }

    fn tables(&self) -> usize {
        self.inner.tables()
    }

    fn memories(&self) -> usize {
        self.inner.memories()
    }
}

#[derive(Default)]
pub struct Log {
    limits: Option<LogLimits>,
    lines: Vec<String>,
    this_tick: u32,
    dropped: u64,
}

impl Log {
    pub(crate) fn push(&mut self, text: &str) {
        let Some(limits) = self.limits else {
            return;
        };
        if self.this_tick >= limits.lines_per_tick {
            self.dropped += 1;
            return;
        }
        self.this_tick += 1;
        let mut end = text.len().min(limits.line_bytes);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        self.lines.push(text[..end].to_string());
    }

    pub(crate) fn tick(&mut self) {
        self.this_tick = 0;
    }

    pub(crate) fn take(&mut self) -> Vec<String> {
        std::mem::take(&mut self.lines)
    }

    pub(crate) fn dropped(&self) -> u64 {
        self.dropped
    }
}

pub struct ModCtx<T> {
    pub game: T,
    pub(crate) limiter: Limiter,
    pub(crate) log: Log,
}

impl<T> ModCtx<T> {
    pub(crate) fn new(game: T, limits: &Limits) -> Self {
        Self {
            game,
            limiter: Limiter::new(limits),
            log: Log {
                limits: limits.log,
                ..Log::default()
            },
        }
    }
}
