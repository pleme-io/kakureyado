# kakureyado

Onion service hosting platform.

Expose any TCP or HTTP service as a Tor `.onion` hidden service. Manages the
full service lifecycle -- key generation, descriptor publishing, backend
load balancing, and vanity address generation. Vanity search runs in parallel
across all CPU cores using ed25519-dalek and rayon.

## Quick Start

```bash
cargo test                   # run all 45 tests
cargo build --release        # release binary
nix build                    # Nix hermetic build
```

## Crates

| Crate | Purpose |
|-------|---------|
| `kakureyado-core` | Traits and types: `OnionServiceHost`, `VanityGenerator`, `ServiceRegistry` |
| `kakureyado-service` | Implementations: local host, brute-force vanity generator, memory registry |
| `kakureyado-cli` | CLI binary with `start`, `stop`, `list`, `vanity`, and `status` subcommands |

## Vanity Address Generation

Generates v3 `.onion` addresses matching a desired prefix. The algorithm derives
`base32(pubkey || checksum || 0x03)` where checksum is the first 2 bytes of
`SHA3-256(".onion checksum" || pubkey || 0x03)`. Expected attempts: `32^prefix_len`.

## Usage

```bash
# Generate a vanity .onion address with prefix "ab"
kakureyado vanity ab

# Generate with a longer prefix (slower, ~32^n attempts)
kakureyado vanity pleme

# Expose a local web server as a .onion service
kakureyado start --name web --port 8080

# List running onion services
kakureyado list

# Check service status
kakureyado status --name web

# Stop a service
kakureyado stop --name web
```

## License

MIT
