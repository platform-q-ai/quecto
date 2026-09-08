use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct SwarmConfig {
    pub default_timeout_seconds: u64,
    pub max_foreground_seconds: u64,
    pub max_background_seconds: u64,
    pub default_max_output_bytes: usize,
    pub max_output_bytes: usize,
    pub max_memory_bytes: Option<u64>,
    pub max_cpu_seconds: Option<u64>,
    pub max_processes: Option<u32>,
    pub max_concurrent_jobs: usize,
    pub inherit_environment: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmToolConfig {
    #[serde(default = "default_swarm_timeout_seconds")]
    pub default_timeout_seconds: u64,
    #[serde(default = "default_swarm_max_foreground_seconds")]
    pub max_foreground_seconds: u64,
    #[serde(default = "default_swarm_max_background_seconds")]
    pub max_background_seconds: u64,
    #[serde(default = "default_swarm_output_bytes")]
    pub default_max_output_bytes: usize,
    #[serde(default = "default_swarm_max_output_bytes")]
    pub max_output_bytes: usize,
    #[serde(default)]
    pub max_memory_bytes: Option<u64>,
    #[serde(default)]
    pub max_cpu_seconds: Option<u64>,
    #[serde(default = "default_swarm_max_processes")]
    pub max_processes: Option<u32>,
    #[serde(default = "default_swarm_concurrent_jobs")]
    pub max_concurrent_jobs: usize,
    #[serde(default)]
    pub inherit_environment: bool,
}

impl Default for SwarmToolConfig {
    fn default() -> Self {
        Self {
            default_timeout_seconds: default_swarm_timeout_seconds(),
            max_foreground_seconds: default_swarm_max_foreground_seconds(),
            max_background_seconds: default_swarm_max_background_seconds(),
            default_max_output_bytes: default_swarm_output_bytes(),
            max_output_bytes: default_swarm_max_output_bytes(),
            max_memory_bytes: None,
            max_cpu_seconds: None,
            max_processes: default_swarm_max_processes(),
            max_concurrent_jobs: default_swarm_concurrent_jobs(),
            inherit_environment: false,
        }
    }
}
impl From<SwarmToolConfig> for SwarmConfig {
    fn from(v: SwarmToolConfig) -> Self {
        Self {
            default_timeout_seconds: v.default_timeout_seconds,
            max_foreground_seconds: v.max_foreground_seconds,
            max_background_seconds: v.max_background_seconds,
            default_max_output_bytes: v.default_max_output_bytes,
            max_output_bytes: v.max_output_bytes,
            max_memory_bytes: v.max_memory_bytes,
            max_cpu_seconds: v.max_cpu_seconds,
            max_processes: v.max_processes,
            max_concurrent_jobs: v.max_concurrent_jobs,
            inherit_environment: v.inherit_environment,
        }
    }
}
fn default_swarm_timeout_seconds() -> u64 {
    60
}
fn default_swarm_max_foreground_seconds() -> u64 {
    300
}
fn default_swarm_max_background_seconds() -> u64 {
    1800
}
fn default_swarm_output_bytes() -> usize {
    200_000
}
fn default_swarm_max_output_bytes() -> usize {
    1_000_000
}
fn default_swarm_max_processes() -> Option<u32> {
    Some(1)
}
fn default_swarm_concurrent_jobs() -> usize {
    2
}
impl Default for SwarmConfig {
    fn default() -> Self {
        SwarmToolConfig::default().into()
    }
}
