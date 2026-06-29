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

The commit flow writes a canonical proof artifact under the chain proof directory, and verification succeeds with the frozen verifier output:

```text
OK: proof valid
```

## Report determinism

The report generation path writes deterministic artifacts under the chain proof directory and produces a stable PDF output for repeated runs.

## Noted mismatches

- The previous README text had drifted from the real CLI contract and still referenced path-based report usage.
- The verify path was previously noisy; it now emits the frozen single-line success output required by the protocol contract.
