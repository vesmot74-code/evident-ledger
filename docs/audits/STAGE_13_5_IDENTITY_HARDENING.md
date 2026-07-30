# Stage 13.5 — Identity Hardening

Date: 2026-07-30  
Status: Hardening applied. No key loss. Pilot identity unchanged.

## Summary

Investigation found a **client-pinned identity mismatch**, not key loss or corruption.

- `SIGNING_KEY_PATH` unset → fallback to `./signing_key.bin`.
- `ServerSigner::load_or_create()` auto-creates a key if the file is missing.
- Dev fallback identity was pinned earlier into `~/.evident/server_identity.pub`.
- Pilot production uses a different signing key.

This stage hardens startup so production cannot silently mint a new key, and so operators can see a SHA-256 fingerprint of the loaded seed at boot.

## Root cause

| Date       | Source            | Public key                                                         |
| ---------- | ----------------- | ------------------------------------------------------------------ |
| 2026-07-14 | `./signing_key.bin` | `81dc1ab20cecbfeb698c77b271e953267e35b4f029b4d1ca89e81a6377397fb9` |
| 2026-07-23 | `pilot116-key`      | `fd97921df83d5e4adfa94f30989e93411f17641770446c91b6adc3f5676b156a` |

Dev fallback SHA-256:

```
4586b00a3c5c0162b3f32701afcbe6ef6754db93d7b9dae6fb3491442177edb2
```

Pilot production SHA-256:

```
f21dbaf7fa6e6e3b94ce657163f7cc72160f332693cdac8d2ad76602b7be622e
```

## Verification

The public key and signing fingerprint are verified through the normal server startup path.

Production key verification:

```bash
ENVIRONMENT=production \
DEV_MODE=false \
SIGNING_KEY_PATH="$(pwd)/target/pilot116-key.JBOhAH/signing_key.bin" \
cargo run
```

Expected startup output:

```
Public key: fd97921df83d5e4adfa94f30989e93411f17641770446c91b6adc3f5676b156a
Signing key SHA256: f21dbaf7fa6e6e3b94ce657163f7cc72160f332693cdac8d2ad76602b7be622e
Environment: production
```

The SHA-256 fingerprint can also be independently checked:

```bash
shasum -a 256 target/pilot116-key.JBOhAH/signing_key.bin
```

Expected:

```
f21dbaf7fa6e6e3b94ce657163f7cc72160f332693cdac8d2ad76602b7be622e
```

The fingerprint is calculated from the raw Ed25519 seed bytes.
It does not expose the private key material.

## Hardening changes

1. **Production does not auto-create** a signing key: missing file → panic with `Production signing key missing: …`.
2. **Startup prints SHA-256 fingerprint** of the raw Ed25519 seed (`sha256_fingerprint()`), matching `shasum -a 256 signing_key.bin`.
3. **Dev fallback warns** when `SIGNING_KEY_PATH` is unset:  
   `WARNING: SIGNING_KEY_PATH is not set. Using local development signing key.`
4. **Pilot identity is unchanged** — no rotation; production continues to load the existing pilot key via explicit `SIGNING_KEY_PATH`.
