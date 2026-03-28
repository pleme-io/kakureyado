//! Tor-backed onion service host using kakuremino's `TorTransport`.
//!
//! Gated behind the `tor` feature. Generates real ed25519 keypairs and derives
//! valid v3 `.onion` addresses. Full arti-based service hosting (accept inbound
//! connections via Tor) is not yet wired — the lifecycle methods manage state and
//! log a TODO for the arti-axum integration layer.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use ed25519_dalek::SigningKey;
use rand::rngs::OsRng;
use sha3::{Digest, Sha3_256};
use tokio::sync::RwLock;
use tracing::info;

use kakureyado_core::{
    Error, OnionService, OnionServiceHost, Result, ServiceConfig, ServiceStatus,
};

/// Internal record tracking a hosted service and its keypair.
#[derive(Debug, Clone)]
struct TorServiceEntry {
    service: OnionService,
    /// Ed25519 signing key retained for future arti-axum integration.
    #[allow(dead_code)]
    signing_key: [u8; 32],
}

/// Tor-backed [`OnionServiceHost`] that generates real ed25519 keypairs and
/// derives valid v3 `.onion` addresses via the same algorithm used in
/// `vanity.rs`.
///
/// The actual inbound Tor hosting (arti-axum) is a TODO — but key generation,
/// `.onion` address derivation, and lifecycle state tracking are fully
/// functional.
#[derive(Debug)]
pub struct TorOnionHost {
    services: Arc<RwLock<HashMap<String, TorServiceEntry>>>,
}

impl TorOnionHost {
    #[must_use]
    pub fn new() -> Self {
        Self {
            services: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

impl Default for TorOnionHost {
    fn default() -> Self {
        Self::new()
    }
}

/// Derive the `.onion` v3 address from an ed25519 public key.
///
/// Identical to the algorithm in `vanity.rs`:
///   `base32(pubkey || checksum || 0x03)` where
///   checksum = first 2 bytes of `SHA3-256(".onion checksum" || pubkey || 0x03)`
fn onion_address_from_pubkey(pubkey: &[u8; 32]) -> String {
    const VERSION: u8 = 0x03;
    const ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";

    let mut hasher = Sha3_256::new();
    hasher.update(b".onion checksum");
    hasher.update(pubkey);
    hasher.update([VERSION]);
    let checksum_full = hasher.finalize();

    let mut addr_bytes = [0u8; 35];
    addr_bytes[..32].copy_from_slice(pubkey);
    addr_bytes[32] = checksum_full[0];
    addr_bytes[33] = checksum_full[1];
    addr_bytes[34] = VERSION;

    // RFC 4648 base32 (lowercase, no padding). 35 bytes = 280 bits = 56 chars.
    let mut out = String::with_capacity(56 + 6);
    let mut buffer: u64 = 0;
    let mut bits_left = 0;
    for &byte in &addr_bytes {
        buffer = (buffer << 8) | u64::from(byte);
        bits_left += 8;
        while bits_left >= 5 {
            bits_left -= 5;
            let idx = ((buffer >> bits_left) & 0x1F) as usize;
            out.push(ALPHABET[idx] as char);
        }
    }
    out.push_str(".onion");
    out
}

#[async_trait]
impl OnionServiceHost for TorOnionHost {
    async fn create_service(&self, config: &ServiceConfig) -> Result<OnionService> {
        let mut services = self.services.write().await;

        if services.contains_key(&config.name) {
            return Err(Error::AlreadyExists(config.name.clone()));
        }

        // Generate a real ed25519 keypair.
        let signing_key = SigningKey::generate(&mut OsRng);
        let pubkey = signing_key.verifying_key().to_bytes();
        let onion_address = onion_address_from_pubkey(&pubkey);

        info!(
            name = %config.name,
            onion_address = %onion_address,
            "created onion service with real ed25519 keypair"
        );

        let service = OnionService {
            name: config.name.clone(),
            onion_address,
            target_addr: config.target_addr.clone(),
            target_port: config.target_port,
            status: ServiceStatus::Stopped,
            created_at: Utc::now(),
        };

        services.insert(
            config.name.clone(),
            TorServiceEntry {
                service: service.clone(),
                signing_key: signing_key.to_bytes(),
            },
        );
        Ok(service)
    }

    async fn start_service(&self, name: &str) -> Result<()> {
        let mut services = self.services.write().await;
        let entry = services
            .get_mut(name)
            .ok_or_else(|| Error::ServiceNotFound(name.to_owned()))?;

        // TODO: Wire up arti onion service hosting (arti-axum) to accept
        // inbound connections on the .onion address and proxy to the target.
        info!(
            name = %name,
            onion_address = %entry.service.onion_address,
            "starting onion service (arti-axum integration TODO)"
        );

        entry.service.status = ServiceStatus::Running;
        Ok(())
    }

    async fn stop_service(&self, name: &str) -> Result<()> {
        let mut services = self.services.write().await;
        let entry = services
            .get_mut(name)
            .ok_or_else(|| Error::ServiceNotFound(name.to_owned()))?;

        info!(name = %name, "stopping onion service");
        entry.service.status = ServiceStatus::Stopped;
        Ok(())
    }

    async fn service_status(&self, name: &str) -> Result<ServiceStatus> {
        let services = self.services.read().await;
        services
            .get(name)
            .map(|e| e.service.status)
            .ok_or_else(|| Error::ServiceNotFound(name.to_owned()))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config(name: &str) -> ServiceConfig {
        ServiceConfig {
            name: name.to_owned(),
            target_addr: "127.0.0.1".to_owned(),
            target_port: 8080,
            onion_port: 80,
            persistent: false,
            vanity_prefix: None,
        }
    }

    #[tokio::test]
    async fn create_generates_valid_onion() {
        let host = TorOnionHost::new();
        let svc = host
            .create_service(&test_config("web"))
            .await
            .expect("create must succeed");

        // Must end with .onion.
        assert!(svc.onion_address.ends_with(".onion"), "address = {}", svc.onion_address);

        // Base32 portion must be exactly 56 characters.
        let base = svc.onion_address.strip_suffix(".onion").unwrap();
        assert_eq!(base.len(), 56, "base32 portion must be 56 chars, got {}", base.len());

        // Must only contain valid base32 characters.
        assert!(
            base.chars().all(|c| c.is_ascii_lowercase() || ('2'..='7').contains(&c)),
            "invalid base32 chars in {base}"
        );
    }

    #[tokio::test]
    async fn start_stop_lifecycle() {
        let host = TorOnionHost::new();
        host.create_service(&test_config("api"))
            .await
            .expect("create");

        assert_eq!(
            host.service_status("api").await.unwrap(),
            ServiceStatus::Stopped,
            "initial status must be Stopped"
        );

        host.start_service("api").await.expect("start");
        assert_eq!(
            host.service_status("api").await.unwrap(),
            ServiceStatus::Running,
            "after start must be Running"
        );

        host.stop_service("api").await.expect("stop");
        assert_eq!(
            host.service_status("api").await.unwrap(),
            ServiceStatus::Stopped,
            "after stop must be Stopped"
        );
    }

    #[tokio::test]
    async fn status_tracking() {
        let host = TorOnionHost::new();
        host.create_service(&test_config("tracker"))
            .await
            .expect("create");

        // Initially stopped.
        assert_eq!(
            host.service_status("tracker").await.unwrap(),
            ServiceStatus::Stopped
        );

        // Start twice — should remain Running.
        host.start_service("tracker").await.expect("start 1");
        host.start_service("tracker").await.expect("start 2");
        assert_eq!(
            host.service_status("tracker").await.unwrap(),
            ServiceStatus::Running
        );

        // Missing service errors.
        let err = host
            .service_status("ghost")
            .await
            .expect_err("missing must fail");
        assert!(matches!(err, Error::ServiceNotFound(_)));
    }

    #[tokio::test]
    async fn duplicate_rejected() {
        let host = TorOnionHost::new();
        host.create_service(&test_config("dup"))
            .await
            .expect("first create");

        let err = host
            .create_service(&test_config("dup"))
            .await
            .expect_err("duplicate must fail");
        assert!(
            matches!(err, Error::AlreadyExists(ref n) if n == "dup"),
            "expected AlreadyExists, got {err:?}"
        );
    }

    #[tokio::test]
    async fn each_service_gets_unique_address() {
        let host = TorOnionHost::new();
        let svc1 = host
            .create_service(&test_config("svc1"))
            .await
            .expect("create svc1");
        let svc2 = host
            .create_service(&test_config("svc2"))
            .await
            .expect("create svc2");

        assert_ne!(
            svc1.onion_address, svc2.onion_address,
            "different services must get different .onion addresses"
        );
    }
}
