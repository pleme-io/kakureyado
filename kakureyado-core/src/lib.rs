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
    /// The requested service was not found in the registry.
    #[error("service not found: {0}")]
    ServiceNotFound(String),

    /// A service with this name already exists.
    #[error("service already exists: {0}")]
    AlreadyExists(String),

    /// Tor bootstrap process failed.
    #[error("tor bootstrap failed: {0}")]
    BootstrapFailed(String),

    /// Vanity address generation exceeded the time limit.
    #[error("vanity generation timed out after {0:?}")]
    VanityTimeout(Duration),

    /// I/O error.
    #[error(transparent)]
    Io(#[from] std::io::Error),

    /// Configuration or persistence error.
    #[error("config error: {0}")]
    Config(String),
}

impl PartialEq for Error {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::ServiceNotFound(a), Self::ServiceNotFound(b))
            | (Self::AlreadyExists(a), Self::AlreadyExists(b))
            | (Self::BootstrapFailed(a), Self::BootstrapFailed(b))
            | (Self::Config(a), Self::Config(b)) => a == b,
            (Self::VanityTimeout(a), Self::VanityTimeout(b)) => a == b,
            (Self::Io(a), Self::Io(b)) => a.kind() == b.kind(),
            _ => false,
        }
    }
}

impl Error {
    /// Whether the error is transient and the operation could be retried.
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::BootstrapFailed(_) | Self::VanityTimeout(_) | Self::Io(_)
        )
    }
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vanity_prefix: Option<String>,
}

/// A running or registered onion service.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OnionService {
    pub name: String,
    pub onion_address: String,
    pub target_addr: String,
    pub target_port: u16,
    pub status: ServiceStatus,
    pub created_at: DateTime<Utc>,
}

/// Result of a vanity address generation run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
// Key hierarchy
// ---------------------------------------------------------------------------

/// Types of cryptographic keys in the Tor v3 onion service key hierarchy.
///
/// Tor v3 onion services use a three-tier key hierarchy: a long-term
/// identity key (ed25519), a medium-term descriptor signing key, and
/// short-term introduction point authentication keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyType {
    /// Long-term identity key (ed25519). Derives the `.onion` address.
    Identity,
    /// Medium-term descriptor signing key, rotated periodically.
    DescriptorSigning,
    /// Short-term introduction point authentication key.
    IntroPointAuth,
}

impl fmt::Display for KeyType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Identity => write!(f, "identity"),
            Self::DescriptorSigning => write!(f, "descriptor_signing"),
            Self::IntroPointAuth => write!(f, "intro_point_auth"),
        }
    }
}

// ---------------------------------------------------------------------------
// Descriptor state
// ---------------------------------------------------------------------------

/// Lifecycle state of an onion service descriptor.
///
/// Tracks the publication state of the hidden service descriptor that
/// is uploaded to the HSDir ring so clients can discover the service.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DescriptorState {
    /// Descriptor has not been published yet.
    #[default]
    Unpublished,
    /// Descriptor upload is in progress.
    Publishing,
    /// Descriptor is live on the HSDir ring.
    Published,
    /// Descriptor has expired and is no longer valid.
    Expired,
    /// Descriptor publication failed.
    Failed,
}

impl fmt::Display for DescriptorState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unpublished => write!(f, "unpublished"),
            Self::Publishing => write!(f, "publishing"),
            Self::Published => write!(f, "published"),
            Self::Expired => write!(f, "expired"),
            Self::Failed => write!(f, "failed"),
        }
    }
}

impl DescriptorState {
    /// Whether the descriptor is currently active (published and not expired).
    #[must_use]
    pub fn is_active(&self) -> bool {
        matches!(self, Self::Published)
    }
}

// ---------------------------------------------------------------------------
// Load balancing
// ---------------------------------------------------------------------------

/// Strategy for distributing traffic across backend onion service instances.
///
/// Modelled after OnionBalance's load balancing capabilities for
/// distributing traffic across multiple backend `.onion` instances
/// behind a single frontend address.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum LoadBalanceStrategy {
    /// Distribute requests evenly in sequential order.
    #[default]
    RoundRobin,
    /// Route to the backend with the fewest active connections.
    LeastConnections,
    /// Choose a random backend for each request.
    Random,
    /// Round-robin weighted by the `weight` field on each backend.
    WeightedRoundRobin,
}

impl fmt::Display for LoadBalanceStrategy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RoundRobin => write!(f, "round_robin"),
            Self::LeastConnections => write!(f, "least_connections"),
            Self::Random => write!(f, "random"),
            Self::WeightedRoundRobin => write!(f, "weighted_round_robin"),
        }
    }
}

/// A backend onion service instance behind an OnionBalance frontend.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BackendInstance {
    /// Human-readable name for this backend.
    pub name: String,
    /// The `.onion` address of this backend instance.
    pub onion_address: String,
    /// Relative weight for weighted round-robin (0 = excluded).
    #[serde(default)]
    pub weight: u32,
    /// Whether this backend is currently reachable.
    #[serde(default)]
    pub healthy: bool,
    /// ISO 8601 timestamp of last successful health check.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_seen: Option<String>,
}

// ---------------------------------------------------------------------------
// Access control
// ---------------------------------------------------------------------------

/// Access control mode for an onion service.
///
/// Tor v3 supports restricting access to a set of authorized clients
/// who hold x25519 keys. `Public` means anyone can connect;
/// `AuthorizedClients` restricts access to pre-shared key holders.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AccessMode {
    /// Service is accessible to anyone who knows the `.onion` address.
    #[default]
    Public,
    /// Only clients with pre-shared x25519 keys can connect.
    AuthorizedClients,
}

impl fmt::Display for AccessMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Public => write!(f, "public"),
            Self::AuthorizedClients => write!(f, "authorized_clients"),
        }
    }
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_status_display() {
        assert_eq!(ServiceStatus::Stopped.to_string(), "stopped");
        assert_eq!(ServiceStatus::Starting.to_string(), "starting");
        assert_eq!(ServiceStatus::Running.to_string(), "running");
        assert_eq!(ServiceStatus::Error.to_string(), "error");
    }

    #[test]
    fn error_display_variants() {
        assert_eq!(
            Error::ServiceNotFound("web".into()).to_string(),
            "service not found: web"
        );
        assert_eq!(
            Error::AlreadyExists("web".into()).to_string(),
            "service already exists: web"
        );
        assert_eq!(
            Error::BootstrapFailed("timeout".into()).to_string(),
            "tor bootstrap failed: timeout"
        );
        assert_eq!(
            Error::Config("bad yaml".into()).to_string(),
            "config error: bad yaml"
        );
    }

    #[test]
    fn error_partial_eq() {
        assert_eq!(
            Error::ServiceNotFound("a".into()),
            Error::ServiceNotFound("a".into())
        );
        assert_ne!(
            Error::ServiceNotFound("a".into()),
            Error::AlreadyExists("a".into())
        );
        assert_eq!(
            Error::VanityTimeout(Duration::from_secs(10)),
            Error::VanityTimeout(Duration::from_secs(10))
        );
    }

    #[test]
    fn error_is_retryable() {
        assert!(Error::BootstrapFailed("fail".into()).is_retryable());
        assert!(Error::VanityTimeout(Duration::from_secs(10)).is_retryable());
        assert!(!Error::ServiceNotFound("web".into()).is_retryable());
        assert!(!Error::AlreadyExists("web".into()).is_retryable());
        assert!(!Error::Config("bad".into()).is_retryable());
    }

    #[test]
    fn service_config_serialization_roundtrip() {
        let config = ServiceConfig {
            name: "web".into(),
            target_addr: "127.0.0.1".into(),
            target_port: 8080,
            onion_port: 80,
            persistent: true,
            vanity_prefix: Some("abc".into()),
        };
        let json = serde_json::to_string(&config).unwrap();
        let deserialized: ServiceConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(config, deserialized);
    }

    #[test]
    fn service_config_skip_none_vanity() {
        let config = ServiceConfig {
            name: "web".into(),
            target_addr: "127.0.0.1".into(),
            target_port: 8080,
            onion_port: 80,
            persistent: false,
            vanity_prefix: None,
        };
        let json = serde_json::to_string(&config).unwrap();
        assert!(!json.contains("vanity_prefix"));
    }

    #[test]
    fn onion_service_serialization_roundtrip() {
        let svc = OnionService {
            name: "web".into(),
            onion_address: "abc.onion".into(),
            target_addr: "127.0.0.1".into(),
            target_port: 8080,
            status: ServiceStatus::Running,
            created_at: Utc::now(),
        };
        let json = serde_json::to_string(&svc).unwrap();
        let deserialized: OnionService = serde_json::from_str(&json).unwrap();
        assert_eq!(svc.name, deserialized.name);
        assert_eq!(svc.status, deserialized.status);
    }

    #[test]
    fn service_status_serialization_roundtrip() {
        for status in [
            ServiceStatus::Stopped,
            ServiceStatus::Starting,
            ServiceStatus::Running,
            ServiceStatus::Error,
        ] {
            let json = serde_json::to_string(&status).unwrap();
            let deserialized: ServiceStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(status, deserialized);
        }
    }

    #[test]
    fn vanity_result_eq() {
        let a = VanityResult {
            address: "abc.onion".into(),
            keypair_path: PathBuf::from("memory"),
            attempts: 100,
            duration: Duration::from_secs(1),
        };
        let b = a.clone();
        assert_eq!(a, b);
    }

    // -- KeyType tests ----------------------------------------------------

    #[test]
    fn key_type_display() {
        assert_eq!(KeyType::Identity.to_string(), "identity");
        assert_eq!(KeyType::DescriptorSigning.to_string(), "descriptor_signing");
        assert_eq!(KeyType::IntroPointAuth.to_string(), "intro_point_auth");
    }

    #[test]
    fn key_type_serialization_roundtrip() {
        let types = vec![
            KeyType::Identity,
            KeyType::DescriptorSigning,
            KeyType::IntroPointAuth,
        ];
        for kt in types {
            let json = serde_json::to_string(&kt).unwrap();
            let deserialized: KeyType = serde_json::from_str(&json).unwrap();
            assert_eq!(kt, deserialized);
        }
    }

    #[test]
    fn key_type_eq() {
        assert_eq!(KeyType::Identity, KeyType::Identity);
        assert_ne!(KeyType::Identity, KeyType::DescriptorSigning);
        assert_ne!(KeyType::DescriptorSigning, KeyType::IntroPointAuth);
    }

    // -- DescriptorState tests --------------------------------------------

    #[test]
    fn descriptor_state_default_is_unpublished() {
        assert_eq!(DescriptorState::default(), DescriptorState::Unpublished);
    }

    #[test]
    fn descriptor_state_display() {
        assert_eq!(DescriptorState::Unpublished.to_string(), "unpublished");
        assert_eq!(DescriptorState::Publishing.to_string(), "publishing");
        assert_eq!(DescriptorState::Published.to_string(), "published");
        assert_eq!(DescriptorState::Expired.to_string(), "expired");
        assert_eq!(DescriptorState::Failed.to_string(), "failed");
    }

    #[test]
    fn descriptor_state_is_active() {
        assert!(!DescriptorState::Unpublished.is_active());
        assert!(!DescriptorState::Publishing.is_active());
        assert!(DescriptorState::Published.is_active());
        assert!(!DescriptorState::Expired.is_active());
        assert!(!DescriptorState::Failed.is_active());
    }

    #[test]
    fn descriptor_state_serialization_roundtrip() {
        let states = vec![
            DescriptorState::Unpublished,
            DescriptorState::Publishing,
            DescriptorState::Published,
            DescriptorState::Expired,
            DescriptorState::Failed,
        ];
        for state in states {
            let json = serde_json::to_string(&state).unwrap();
            let deserialized: DescriptorState = serde_json::from_str(&json).unwrap();
            assert_eq!(state, deserialized);
        }
    }

    // -- LoadBalanceStrategy tests ----------------------------------------

    #[test]
    fn load_balance_strategy_default_is_round_robin() {
        assert_eq!(
            LoadBalanceStrategy::default(),
            LoadBalanceStrategy::RoundRobin
        );
    }

    #[test]
    fn load_balance_strategy_display() {
        assert_eq!(LoadBalanceStrategy::RoundRobin.to_string(), "round_robin");
        assert_eq!(
            LoadBalanceStrategy::LeastConnections.to_string(),
            "least_connections"
        );
        assert_eq!(LoadBalanceStrategy::Random.to_string(), "random");
        assert_eq!(
            LoadBalanceStrategy::WeightedRoundRobin.to_string(),
            "weighted_round_robin"
        );
    }

    #[test]
    fn load_balance_strategy_serialization_roundtrip() {
        let strategies = vec![
            LoadBalanceStrategy::RoundRobin,
            LoadBalanceStrategy::LeastConnections,
            LoadBalanceStrategy::Random,
            LoadBalanceStrategy::WeightedRoundRobin,
        ];
        for strategy in strategies {
            let json = serde_json::to_string(&strategy).unwrap();
            let deserialized: LoadBalanceStrategy = serde_json::from_str(&json).unwrap();
            assert_eq!(strategy, deserialized);
        }
    }

    // -- BackendInstance tests --------------------------------------------

    #[test]
    fn backend_instance_serialization_roundtrip() {
        let instance = BackendInstance {
            name: "backend-1".into(),
            onion_address: "abc123.onion".into(),
            weight: 5,
            healthy: true,
            last_seen: Some("2026-01-01T00:00:00Z".into()),
        };
        let json = serde_json::to_string(&instance).unwrap();
        let deserialized: BackendInstance = serde_json::from_str(&json).unwrap();
        assert_eq!(instance, deserialized);
    }

    #[test]
    fn backend_instance_last_seen_skipped_when_none() {
        let instance = BackendInstance {
            name: "backend-2".into(),
            onion_address: "xyz789.onion".into(),
            weight: 0,
            healthy: false,
            last_seen: None,
        };
        let json = serde_json::to_string(&instance).unwrap();
        assert!(!json.contains("last_seen"));
    }

    #[test]
    fn backend_instance_defaults() {
        // Deserialize with only required fields to verify serde defaults.
        let json = r#"{"name":"b","onion_address":"x.onion"}"#;
        let instance: BackendInstance = serde_json::from_str(json).unwrap();
        assert_eq!(instance.weight, 0);
        assert!(!instance.healthy);
        assert!(instance.last_seen.is_none());
    }

    // -- AccessMode tests -------------------------------------------------

    #[test]
    fn access_mode_default_is_public() {
        assert_eq!(AccessMode::default(), AccessMode::Public);
    }

    #[test]
    fn access_mode_display() {
        assert_eq!(AccessMode::Public.to_string(), "public");
        assert_eq!(
            AccessMode::AuthorizedClients.to_string(),
            "authorized_clients"
        );
    }

    #[test]
    fn access_mode_serialization_roundtrip() {
        let modes = vec![AccessMode::Public, AccessMode::AuthorizedClients];
        for mode in modes {
            let json = serde_json::to_string(&mode).unwrap();
            let deserialized: AccessMode = serde_json::from_str(&json).unwrap();
            assert_eq!(mode, deserialized);
        }
    }

    #[test]
    fn access_mode_eq() {
        assert_eq!(AccessMode::Public, AccessMode::Public);
        assert_ne!(AccessMode::Public, AccessMode::AuthorizedClients);
    }
}
