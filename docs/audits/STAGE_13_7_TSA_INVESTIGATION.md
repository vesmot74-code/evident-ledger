# Stage 13.7 — TSA Investigation Log

**Status:** diagnosis complete — no code/test changes in this step  
**Date:** 2026-07-30  
**Branch:** `stage-13.7-release-candidate`  
**Scope:** investigate `legacy_events_signature_persist` failure only  
**Non-goals:** implement fixes, modify production code, modify tests, decide RC approval

**Document role:** working investigation journal.  
**RC decision source of truth:** `docs/audits/STAGE_13_7_RELEASE_CANDIDATE.md` (`## Known Limitations` + eventual RC decision).

---

## 1. Objective

Investigate why:

```bash
cargo test --test legacy_events_signature_persist -- --nocapture
```

fails with:

- `v1_events_persists_response_signature_exactly` → `expect("proof signature")`
- `legacy_events_anchors_and_materializes_public_proof` → `proof_status` `Some("failed")` vs `Some("anchored")`

Server availability is not the cause (in-process Axum test app).

---

## 2. Step 1 — TSA test targets

```text
tests/tsa_read_verify.rs   # present
```

`find` for `*tsa*test*` / `*test*tsa*` under repo (excluding `target`) also surfaces vendor/smoke-related names; primary integration coverage for read-path is `tsa_read_verify`.

### `cargo test --test tsa_read_verify -- --nocapture`

```text
running 4 tests
stub_outside_dev_fails ... ok
malformed_der_fails ... ok
stub_in_dev_verifies_then_cached_on_repeat ... ok
changed_token_sha_forces_reverify_not_stale_cache ... ok

test result: ok. 4 passed; 0 failed
```

These tests exercise stub / cache / DER-hook paths and do **not** exercise live FreeTSA OpenSSL `-data` vs digest-imprint semantics against a real DER token.

No new tests were created.

---

## 3. Step 2 — TSA environment scope

### Local environment (checked)

At investigation start, shell had:

```text
FREETSA_CA_CERT_PATH=/tmp/freetsa-trust/cacert.pem
FREETSA_UNTRUSTED_CERT_PATH=/tmp/freetsa-trust/tsa.crt
```

These are developer smoke trust paths (see `docs/audits/TSA_HARDENING_COMPLETION.md`), not application config files.

### Repository (checked)

| Location | `FREETSA_CA_CERT_PATH` / `FREETSA_UNTRUSTED_CERT_PATH` |
|---|---|
| `.env` / `.env.example` | not present |
| `docker-compose.yml` / Dockerfile* | not present |
| `*.toml` app config | not present as deploy defaults |
| Code / ADR / audit docs | documented as optional env for OpenSSL verify |

### Scope boundary (do not overclaim)

No evidence of `FREETSA_*` configuration was found in the repository’s env/docker config files.  
Local shell had developer `FREETSA_*` paths set during this investigation.  
**Deployment/CI environments were not checked.**  
No production exposure is currently expected because this project is still in pre-pilot / RC preparation stage.

---

## 4. Step 3 — Reproduce both verification paths

### Without TSA CA configuration

```bash
env -u FREETSA_CA_CERT_PATH -u FREETSA_UNTRUSTED_CERT_PATH \
  cargo test --test legacy_events_signature_persist -- --nocapture
```

```text
legacy_events_persists_response_signature_exactly ... ok
v1_events_persists_response_signature_exactly ... ok
legacy_events_anchors_and_materializes_public_proof ... ok
test result: ok. 3 passed; 0 failed
```

When CA env is absent, `freetsa_trust_paths()` returns `None` → OpenSSL step skipped → `TsaVerificationStatus::Unavailable` → does **not** set TSA failure signal → proof can remain `anchored`. This **masks** the OpenSSL imprint bug.

### With TSA CA configuration

```bash
export FREETSA_CA_CERT_PATH=/tmp/freetsa-trust/cacert.pem
export FREETSA_UNTRUSTED_CERT_PATH=/tmp/freetsa-trust/tsa.crt
cargo test --test legacy_events_signature_persist -- --nocapture
```

```text
legacy_events_persists_response_signature_exactly ... ok
v1_events_persists_response_signature_exactly ... FAILED
  panicked at ... expect("proof signature")
legacy_events_anchors_and_materializes_public_proof ... FAILED
  left: Some("failed")
  right: Some("anchored")
test result: FAILED. 1 passed; 2 failed
```

TSA stamping still succeeds (log lines `TSA: stamped chain … root …`). Failure is on **read/verify**, not stamp.

### Difference

| Mode | OpenSSL CA verify | Observed proof |
|---|---|---|
| No `FREETSA_*` | skipped → `unavailable` | tests pass (`anchored` + signature present) |
| With `FREETSA_*` | runs `openssl ts -verify -data` → imprint mismatch → `failed` | `proof_status=failed`; failed envelope omits `signature` |

---

## 5. Step 4 — Verification flow trace

### Files inspected

- `src/tsa_worker.rs`
- `src/tsa/read_verify.rs`
- `vendor/notary-tsa/src/openssl_provider.rs`
- `src/api/v1/proof_state.rs`
- `src/api/v1/proof_material.rs`

### Q1 — What bytes are stamped?

Write path (`stamp_chain`):

1. Decode merkle root hex → 32 raw SHA-256 bytes.
2. `Rfc3161Client::timestamp(&hash_bytes)` builds RFC3161 TSQ with those bytes as **message imprint** (digest), not as a document body to be hashed again.

### Q2 — What bytes are verified?

Read path (`verify_rfc3161_token`):

1. `parse_and_validate_tsr(token, &hash)` — structural + imprint against the same 32-byte merkle root (**correct**; matches write).
2. If `FREETSA_*` present: `verify_tsr_bytes(token, &hash, …)` → writes raw 32 bytes to a temp file via `write_digest`, then OpenSSL:

```text
openssl ts -verify -in <tsr> -data <digest_file> -CAfile … -untrusted …
```

OpenSSL `-data` hashes the **file contents** with SHA-256 and compares that hash to the token imprint. The file already contains a hash, so OpenSSL computes `SHA256(merkle_root_bytes)` ≠ `merkle_root` → **message imprint mismatch**.

### Q3 — `-data` or `-digest`?

**Current production read-path OpenSSL call uses `-data`.**

Correct alignment with write-path imprint semantics would be:

```text
openssl ts -verify -digest <merkle_root_hex> …
```

### Q4 — Identical write/read RFC3161 imprint semantics?

| Stage | Semantics | Match? |
|---|---|---|
| Write stamp | imprint = merkle root digest | — |
| Read structural (`parse_and_validate_tsr`) | imprint = merkle root digest | yes |
| Read OpenSSL CA (`verify_tsr_bytes`) | `-data` re-hashes 32-byte file | **no** |

### Failure location (API)

1. OpenSSL verify fails → `TsaVerificationStatus::Failed`
2. `proof_state` maps that to `TsaStatus::Failed` → `failure_signal`
3. `derive_proof_status` → `ProofStatus::Failed`
4. `build_proof_response` failed envelope omits `signature` → test `expect("proof signature")` and `proof_status != anchored`

### Independent OpenSSL probe (same FreeTSA trust material)

For merkle root  
`27377e7422ee49569b8b3f3030a01cda44075ca9453f81d2abbff098212bbf25`  
(token obtained via FreeTSA with `-digest` query):

| Command | Result |
|---|---|
| `openssl ts -verify … -data digest.bin` (32 raw bytes) | `message imprint mismatch` / `Verification: FAILED` |
| `openssl ts -verify … -digest <hex>` | `Verification: OK` |

---

## 6. Step 5 — Regression history (cheap)

| Path | Notable commits |
|---|---|
| `src/tsa/read_verify.rs` | introduced in `d9c57ac` — `fix(tsa): harden RFC3161 verification path` |
| `vendor/notary-tsa/src/openssl_provider.rs` | `d9c57ac` adds/extends `verify_tsr_bytes` using `-data` |
| `tests/legacy_events_signature_persist.rs` | `8194f6c` — signature persist (pre-dates OpenSSL CA read path) |
| Related | `9f69830` — TSA validation into proof failure signal |

**Assessment:** regression appearance aligns with TSA hardening (`d9c57ac`), which enabled full CA-backed OpenSSL verify on read when `FREETSA_*` is set. Prior behavior either skipped crypto verify for FreeTSA DER on read, or skipped OpenSSL when CA env absent (`unavailable`). No expensive bisect run.

---

## 7. Step 6 — Classification

### **A — Production logic regression**

Verification implementation mismatch between write-path digest imprint and read-path OpenSSL `-data` verify.

**Not B:** production OpenSSL verify path is incorrect for digest-imprinted tokens; tests correctly surface bad `proof_status` / missing signature when CA verify runs.

**Not C alone:** docs/ADR mention OpenSSL verify + env vars, but the concrete CLI flag (`-data` vs `-digest`) is the logic bug; clarifying docs alone would not make CA-backed verify succeed for valid tokens.

### Evidence summary

1. Repro: 3/3 pass without `FREETSA_*`; 2/3 fail with `FREETSA_*`.
2. Stamping succeeds; read verify fails.
3. Structural imprint check can succeed while OpenSSL `-data` fails.
4. Same token: `-digest` OK, `-data` imprint mismatch.
5. Introducing change: `d9c57ac` read-path OpenSSL verify.

---

## 8. Deliverable answers

| # | Answer |
|---|---|
| **Root cause** | Read-path OpenSSL uses `ts -verify -data` on a file of raw merkle-root bytes, causing double-hash / imprint mismatch for tokens stamped with digest imprint. |
| **Failure location** | `vendor/notary-tsa/src/openssl_provider.rs` (`openssl_verify_output` / `verify_tsr_bytes`) → `src/tsa/read_verify.rs` → `proof_state` failure_signal → `proof_material` failed envelope without `signature`. |
| **Evidence** | Controlled with/without CA repro; OpenSSL CLI probe; code path inspection; commit `d9c57ac`. |
| **Regression history** | Appears after TSA hardening (`d9c57ac`); failure signal integration (`9f69830`) makes failed verify visible as `proof_status=failed`. |
| **Proposed fix direction** *(not implemented)* | Align OpenSSL verify with write-path: use `-digest <hex>` (or equivalent) instead of `-data` on raw digest bytes; update `write_digest` / ADR wording; re-run `legacy_events_signature_persist` with and without `FREETSA_*`. Alternative: document that CA verify must not use `-data` for digest-imprint tokens. Separate decision: fix code / adjust verification model / update documentation. |

---

## 9. Stop

Investigation complete. No production code, tests, or RC approval decision in this step.  
Next step (separate): choose fix approach and sync final wording into `STAGE_13_7_RELEASE_CANDIDATE.md`.

---

## Resolution

Status: Resolved

Resolution commit:

5681d3c

Changed component:

vendor/notary-tsa/src/openssl_provider.rs

Final verification:

CA-backed RFC3161 verification returns:

verification_status = verified

No test expectations were changed.

No API contract changes introduced.

---

## Follow-up: trust material configuration hardening

### Why prior `unavailable` was ambiguous

Before trust-config hardening, missing `FREETSA_CA_CERT_PATH` /
`FREETSA_UNTRUSTED_CERT_PATH` (or non-existent files) caused
`freetsa_trust_paths() = None` → `verification_status = unavailable`.

That status also reads as “TSA temporarily unreachable”, so a **deployment
misconfiguration** was indistinguishable from an external TSA / network outage
in API responses and audits.

### Digest verification confirmation

RFC3161 read-path verification remains:

`openssl ts -verify -digest <merkle_root_hex>`

There is no return to `-data` for digest-imprint tokens (fix `5681d3c`).

### New trust configuration model

- Pure check: `check_tsa_configuration(ca_path, untrusted_path)` — no env reads.
- Env parsing stays in callers (`freetsa_trust_path_options_from_env` / `main`).
- Production (`ENVIRONMENT=production` or `APP_ENV=production`): invalid trust
  config → controlled `exit(1)` at startup.
- Non-production: startup continues with a tracing warning.

### Distinguishing failure classes

| Class | `verification_status` | `verification_reason` (additive) |
|---|---|---|
| Crypto / imprint reject | `failed` | `verification_failed` |
| Missing trust paths | `unavailable` | `trust_material_missing` |
| Paths set but not files | `unavailable` | `trust_material_invalid` |
| External TSA / network (reserved) | `unavailable` | `tsa_network_unavailable` |

`verification_status` vocabulary and DB cache column values are unchanged.
`verification_reason` is optional on proof TSA JSON only (backward compatible).
