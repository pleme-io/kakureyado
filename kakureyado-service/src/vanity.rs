use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use ed25519_dalek::SigningKey;
use rand::rngs::OsRng;
use rayon::prelude::*;
use sha3::{Digest, Sha3_256};

use kakureyado_core::{Error, Result, VanityGenerator, VanityResult};

/// Generates vanity `.onion` v3 addresses via parallel brute-force search.
#[derive(Debug, Clone)]
pub struct BruteForceVanityGenerator {
    /// Estimated key generations per second (used for time estimates).
    keys_per_second: f64,
}

impl BruteForceVanityGenerator {
    /// Create a new generator with the given performance estimate.
    #[must_use]
    pub fn new(keys_per_second: f64) -> Self {
        Self { keys_per_second }
    }
}

impl Default for BruteForceVanityGenerator {
    fn default() -> Self {
        // Conservative default — actual throughput varies by hardware.
        Self::new(50_000.0)
    }
}

/// Derive the `.onion` v3 address from an ed25519 public key.
///
/// The address is `base32(pubkey || checksum || version)` where:
///   - checksum = first 2 bytes of SHA3-256(b".onion checksum" || pubkey || version)
///   - version  = 0x03
fn onion_address_from_pubkey(pubkey: &[u8; 32]) -> String {
    const VERSION: u8 = 0x03;

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

    let encoded = data_encoding_hex_lowercase(addr_bytes);
    format!("{encoded}.onion")
}

/// RFC 4648 base32 encode (lowercase, no padding).
fn data_encoding_hex_lowercase(input: [u8; 35]) -> String {
    const ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";
    let mut out = String::with_capacity(56);
    let mut buffer: u64 = 0;
    let mut bits_left = 0;

    for &byte in &input {
        buffer = (buffer << 8) | u64::from(byte);
        bits_left += 8;
        while bits_left >= 5 {
            bits_left -= 5;
            let idx = ((buffer >> bits_left) & 0x1F) as usize;
            out.push(ALPHABET[idx] as char);
        }
    }
    // 35 bytes = 280 bits, 280 / 5 = 56 — no remainder.
    out
}

#[async_trait]
impl VanityGenerator for BruteForceVanityGenerator {
    async fn generate(&self, prefix: &str) -> Result<VanityResult> {
        let prefix = prefix.to_lowercase();
        let keys_per_second = self.keys_per_second;

        // Offload CPU-heavy work to rayon.
        let result = tokio::task::spawn_blocking(move || {
            let found = Arc::new(AtomicBool::new(false));
            let total_attempts = Arc::new(AtomicU64::new(0));
            let start = Instant::now();

            // Empty prefix — any key matches.
            if prefix.is_empty() {
                let signing_key = SigningKey::generate(&mut OsRng);
                let pubkey = signing_key.verifying_key().to_bytes();
                let address = onion_address_from_pubkey(&pubkey);
                return Ok(VanityResult {
                    address,
                    keypair_path: PathBuf::from("memory"),
                    attempts: 1,
                    duration: start.elapsed(),
                });
            }

            let num_threads = rayon::current_num_threads();
            let results: Vec<Option<(String, u64)>> = (0..num_threads)
                .into_par_iter()
                .map(|_| {
                    let mut local_attempts: u64 = 0;
                    loop {
                        if found.load(Ordering::Relaxed) {
                            return None;
                        }

                        let signing_key = SigningKey::generate(&mut OsRng);
                        let pubkey = signing_key.verifying_key().to_bytes();
                        let address = onion_address_from_pubkey(&pubkey);
                        local_attempts += 1;

                        // Check if the address (without .onion) starts with the prefix.
                        if address.starts_with(&prefix) {
                            found.store(true, Ordering::Relaxed);
                            total_attempts.fetch_add(local_attempts, Ordering::Relaxed);
                            return Some((address, local_attempts));
                        }

                        // Periodically update global counter (reduce contention).
                        if local_attempts % 1000 == 0 {
                            total_attempts.fetch_add(1000, Ordering::Relaxed);
                        }
                    }
                })
                .collect();

            let (address, _) = results
                .into_iter()
                .flatten()
                .next()
                .expect("at least one thread must find a result");

            let attempts = total_attempts.load(Ordering::Relaxed);
            let duration = start.elapsed();

            tracing::info!(
                prefix,
                attempts,
                elapsed_ms = duration.as_millis(),
                keys_per_second = keys_per_second,
                "vanity address generated"
            );

            Ok(VanityResult {
                address,
                keypair_path: PathBuf::from("memory"),
                attempts,
                duration,
            })
        })
        .await
        .map_err(|e| Error::Config(format!("vanity generation task failed: {e}")))?;

        result
    }

    fn estimate_time(&self, prefix_len: usize) -> Duration {
        if prefix_len == 0 {
            return Duration::ZERO;
        }
        // base32 alphabet has 32 characters — expected attempts = 32^prefix_len.
        let expected_attempts = 32_f64.powi(prefix_len as i32);
        let seconds = expected_attempts / self.keys_per_second;
        Duration::from_secs_f64(seconds)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn onion_address_is_56_chars() {
        let signing_key = SigningKey::generate(&mut OsRng);
        let pubkey = signing_key.verifying_key().to_bytes();
        let addr = onion_address_from_pubkey(&pubkey);
        // 56 base32 chars + ".onion"
        assert!(addr.ends_with(".onion"));
        let base = addr.strip_suffix(".onion").unwrap();
        assert_eq!(base.len(), 56, "base32 portion must be 56 chars");
    }

    #[tokio::test]
    async fn empty_prefix_always_succeeds() {
        let generator = BruteForceVanityGenerator::default();
        let result = generator.generate("").await.expect("empty prefix must succeed");
        assert!(result.address.ends_with(".onion"));
        assert_eq!(result.attempts, 1);
    }

    #[tokio::test]
    async fn generates_valid_onion_with_prefix() {
        let generator = BruteForceVanityGenerator::new(100_000.0);
        // Single-char prefix — fast to find.
        let result = generator.generate("a").await.expect("single-char prefix must succeed");
        assert!(result.address.starts_with('a'));
        assert!(result.address.ends_with(".onion"));
    }

    #[test]
    fn estimate_grows_exponentially() {
        let generator = BruteForceVanityGenerator::new(100_000.0);
        let t1 = generator.estimate_time(1);
        let t2 = generator.estimate_time(2);
        let t3 = generator.estimate_time(3);
        // Each additional char multiplies expected time by 32.
        assert!(t2 > t1, "2-char must take longer than 1-char");
        assert!(t3 > t2, "3-char must take longer than 2-char");
        // Roughly 32x factor.
        let ratio = t2.as_secs_f64() / t1.as_secs_f64();
        assert!(
            (ratio - 32.0).abs() < 1.0,
            "ratio should be ~32, got {ratio}"
        );
    }
}
