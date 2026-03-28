# Kakureyado (隠れ宿) — Onion Service Hosting Platform

Expose any TCP/HTTP service as a `.onion` address. Production onion hosting with
vanity address generation, service lifecycle management, and planned SeaORM persistence.

## Architecture

Three-crate workspace:

```
kakureyado-core     — traits + types (OnionServiceHost, VanityGenerator, ServiceRegistry)
kakureyado-service  — implementations (LocalOnionHost, BruteForceVanityGenerator, MemoryRegistry)
kakureyado-cli      — clap CLI (start, stop, list, vanity, status)
```

## Key Files

| Path | Purpose |
|------|---------|
| `kakureyado-core/src/lib.rs` | Core traits, error types, value structs |
| `kakureyado-service/src/vanity.rs` | Parallel brute-force vanity .onion address generator |
| `kakureyado-service/src/host.rs` | In-memory onion service host (stub) |
| `kakureyado-service/src/registry.rs` | In-memory service registry |
| `kakureyado-cli/src/main.rs` | CLI entry point |

## Vanity Generation Algorithm

1. Generate random ed25519 keypairs using `ed25519-dalek` + `OsRng`
2. Derive v3 `.onion` address: `base32(pubkey || checksum || 0x03)` where
   checksum = first 2 bytes of `SHA3-256(".onion checksum" || pubkey || 0x03)`
3. Check if the base32 address starts with the desired prefix
4. Parallel search via `rayon` across all CPU cores
5. Expected attempts: `32^prefix_len` (base32 alphabet)

## Build Commands

```bash
cargo check                    # Type-check workspace
cargo test                     # Run all tests
cargo build --release          # Release build
cargo run -- vanity ab         # Generate vanity address with prefix "ab"
cargo run -- start -n web -p 8080  # Start a service
```

## Conventions

- Edition 2024, Rust 1.89.0+, MIT license
- clippy pedantic, release profile (lto, strip, opt-level z)
- Pure Rust only — no C FFI (rustls, not native-tls)
- shikumi for config, sea-orm for persistence (planned)
- Nix build via substrate `rust-workspace-release-flake.nix`
