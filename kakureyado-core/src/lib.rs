use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

/// Errors produced by kakureyado operations.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("service not found: {0}")]
    ServiceNotFound(String),

    #[error("service already exists: {0}")]
    AlreadyExists(String),

    #[error("tor bootstrap failed: {0}")]
    BootstrapFailed(String),

    #[error("vanity generation timed out after {0:?}")]
    VanityTimeout(Duration),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error("config error: {0}")]
    Config(String),
}

/// Convenience alias used throughout the crate.
pub type Result<T> = std::result::Result<T, Error>;

// ---------------------------------------------------------------------------
// Value types
// ---------------------------------------------------------------------------

/// Status of a managed onion service.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceStatus {
    Stopped,
    Starting,
    Running,
    Error,
}

impl fmt::Display for ServiceStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stopped => write!(f, "stopped"),
            Self::Starting => write!(f, "starting"),
            Self::Running => write!(f, "running"),
            Self::Error => write!(f, "error"),
        }
    }
}

/// Configuration for creating an onion service.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceConfig {
    /// Human-readable name for the service.
    pub name: String,
    /// Address of the upstream target (e.g. `127.0.0.1`).
    pub target_addr: String,
    /// Port on the upstream target.
    pub target_port: u16,
    /// Port exposed on the .onion address.
    pub onion_port: u16,
    /// Whether to persist the key material across restarts.
    pub persistent: bool,
    /// Optional vanity prefix (lowercase base32 chars).
    pub vanity_prefix: Option<String>,
}

/// A running or registered onion service.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OnionService {
    pub name: String,
    pub onion_address: String,
    pub target_addr: String,
    pub target_port: u16,
    pub status: ServiceStatus,
    pub created_at: DateTime<Utc>,
}

/// Result of a vanity address generation run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VanityResult {
    /// The generated `.onion` address.
    pub address: String,
    /// Filesystem path to the generated keypair.
    pub keypair_path: PathBuf,
    /// Number of keypairs attempted before finding a match.
    pub attempts: u64,
    /// Wall-clock time spent generating.
    pub duration: Duration,
}

// ---------------------------------------------------------------------------
// Traits
// ---------------------------------------------------------------------------

/// Manages the lifecycle of onion services.
#[async_trait]
pub trait OnionServiceHost: Send + Sync {
    async fn create_service(&self, config: &ServiceConfig) -> Result<OnionService>;
    async fn start_service(&self, name: &str) -> Result<()>;
    async fn stop_service(&self, name: &str) -> Result<()>;
    async fn service_status(&self, name: &str) -> Result<ServiceStatus>;
}

/// Generates vanity `.onion` addresses matching a given prefix.
#[async_trait]
pub trait VanityGenerator: Send + Sync {
    async fn generate(&self, prefix: &str) -> Result<VanityResult>;
    fn estimate_time(&self, prefix_len: usize) -> Duration;
}

/// Registry of known onion services.
#[async_trait]
pub trait ServiceRegistry: Send + Sync {
    async fn list(&self) -> Result<Vec<OnionService>>;
    async fn get(&self, name: &str) -> Result<OnionService>;
    async fn register(&self, service: OnionService) -> Result<()>;
    async fn unregister(&self, name: &str) -> Result<()>;
}
