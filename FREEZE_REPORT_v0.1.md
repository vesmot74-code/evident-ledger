# FREEZE REPORT v0.1

## Verified commands

- `cargo test --lib`
- `cargo run --bin evident -- help`
- `cargo run --bin evident -- new-chain`
- `cargo run --bin evident -- commit Cargo.toml --chain 11111111-1111-1111-1111-111111111111`
- `cargo run --bin evident -- verify ~/.evident/proofs/11111111-1111-1111-1111-111111111111/proof.json`
- `cargo run --bin evident -- status 11111111-1111-1111-1111-111111111111`
- `cargo run --bin evident -- report generate 11111111-1111-1111-1111-111111111111`

## CLI stability

The executable contract currently supports the frozen command surface:

- init
- new-chain
- commit
- verify
- status
- report generate

The help output now reports the same contract and does not advertise the old path-based report mode.

## Proof consistency

The commit flow writes a canonical proof artifact under the chain proof directory and verification succeeds with the frozen verifier output:

```text
OK: proof valid
```

## Report determinism

The report generation path writes deterministic artifacts under the chain proof directory and produces a stable PDF output for repeated runs.

## Noted mismatches

* The previous README text had drifted from the real CLI contract and still referenced path-based report usage.
* The verify path was previously noisy; it now emits the frozen single-line success output required by the protocol contract.

---

# Phase 2 Freeze Report

## Zero-file custody model

Commit:

bbc5780

Tag:

v0.2.0-phase2

Status:

COMPLETED

## Architectural changes

The storage model was migrated from automatic file custody to a zero-file custody model.

Implemented changes:

* Removed legacy `originals/` storage directory.
* Removed automatic copying of user documents.
* Removed `persist_original()` workflow.
* Added optional user-controlled local copy references through `proofs/local_copies.json`.
* Updated local integrity verification to validate files only through explicit user-provided paths.

## Storage contract

Current project layout:

<Evident Project>/
├── project.json
├── proofs/
│   ├── <event_id>.json
│   └── local_copies.json
└── audit/
└── audit.jsonl

The system stores cryptographic evidence and references.

Original files remain under user control.

## Validation

Verified:

persist_original removed: PASS

originals directory creation removed: PASS

cargo build -p evident-gui: PASS

Manual GUI scenarios:

Scenario A:
No local copy saved
Result: NOT STORED

Scenario B:
Local copy saved through local_copies.json
Result: VALID

Scenario C:
Local file modified
Result: TAMPERED

## Security impact

The system no longer assumes custody of original evidence files.

Integrity verification is based on:

File hash
|
|
Event record
|
|
Cryptographic proof

This reduces storage risk and preserves user-controlled evidence custody.


