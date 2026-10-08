#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fuel {
    PerCall(u64),
    PerTick(u64),
}

impl Fuel {
    pub fn budget(self) -> u64 {
        match self {
            Self::PerCall(n) | Self::PerTick(n) => n,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LogLimits {
    pub lines_per_tick: u32,
    pub line_bytes: usize,
}

impl Default for LogLimits {
    fn default() -> Self {
        Self {
            lines_per_tick: 16,
            line_bytes: 256,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub fuel: Fuel,
    pub memory_bytes: usize,
    pub table_elements: usize,
    pub instances: usize,
    pub tables: usize,
    pub memories: usize,
    pub stack_bytes: usize,
    pub depth: u32,
    pub component_bytes: usize,
    pub manifest_bytes: usize,
    pub log: Option<LogLimits>,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            fuel: Fuel::PerCall(10_000_000),
            memory_bytes: 64 << 20,
            table_elements: 10_000,
            instances: 32,
            tables: 16,
            memories: 4,
            stack_bytes: 512 << 10,
            depth: 1_000,
            component_bytes: 16 << 20,
            manifest_bytes: 16 << 10,
            log: None,
        }
    }
}
