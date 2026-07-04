# Evident Ledger (v0.1 FROZEN)

Deterministic verifiable event ledger with cryptographic proofs and offline verification.

## System overview

Evident Ledger is a cryptographic event system where:

- every action is an immutable event
- events form a hash-linked chain
- trust is derived from cryptographic proof, not server state
- all results are reproducible offline

## Core pipeline

```text
file → SHA256 → event → chain → proof → verify → report
```

## Quick start

### Build the CLI

```bash
cargo build --bin evident
```

### Initialize local identity

```bash
evident init
```

### Create a new chain

```bash
evident new-chain
```

### Commit a file into a chain

```bash
evident commit <file> --chain <chain_id>
```

Example:

```bash
evident new-chain
# → prints: chain created / chain_id: <generated-uuid>

evident commit Cargo.toml --chain <paste-the-generated-chain_id-here>
evident commit Cargo.toml --chain "$CHAIN_ID"
```

### Verify a proof offline

```bash
evident verify ~/.evident/proofs/<chain_id>/proof.json
```

Expected output:

```text
OK: proof valid
```

### Generate a deterministic report

```bash
evident report generate <chain_id>
```

Artifacts written to:

```text
~/.evident/proofs/<chain_id>/
  ├── proof.json
  └── proof.pdf
```

### Check chain status

```bash
evident status <chain_id>
```

## CLI contract

The frozen workflow is:

- init
- new-chain
- commit
- verify
- status
- report generate

The current executable also exposes auxiliary commands `help` and `hash`, which are preserved for compatibility but are not part of the frozen protocol contract.

## Architecture

```text
Ledger Engine   → immutable event chain
Verifier        → offline cryptographic validation
TSA Layer       → timestamp attestation authority
Report Engine   → deterministic proof exporter
CLI             → orchestration layer (no business logic)
```

## Freeze rules

1. Append-only system
   - Events are immutable and cannot be modified or deleted.
2. Deterministic hashing
   - All hashes use SHA-256 only.
3. Chain integrity
   - Each event is linked to the prior event and the canonical proof is derived from the chain.
4. Offline verification
   - Verification must work without server access.
5. Server is not truth
   - Truth is derived from cryptographic proof, not server state.

## Proof model

The canonical proof artifact contains:

```text
chain_id
root_hash
tsa_timestamp
tsa_signature
event_count
verification_status
```

## Output guarantee

Given identical input:

- proof.json is identical
- proof.pdf is byte-identical
- verification result is identical

Determinism is a hard requirement of the system.

## Tests

```bash
cargo test --lib
```

## Freeze status

- CLI: stable
- Ledger: stable
- Verifier: stable
- Report engine: integrated
- Protocol: FROZEN v0.1

