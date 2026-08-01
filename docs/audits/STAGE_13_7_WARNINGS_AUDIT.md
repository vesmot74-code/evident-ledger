# Stage 13.7 Warnings Audit

## Baseline

| Field | Value |
|-------|-------|
| Branch | `stage-13.7-release-candidate` |
| Commit SHA | `aaa9d3e03373248b1f8ee9d0d235d41d868a423d` |
| Commit subject | `docs: align operational docs with typed startup exit codes` |
| `git status` | clean (matches `origin/stage-13.7-release-candidate`) |
| Date | 2026-08-02 |
| Toolchain | `rustc 1.96.0` / `cargo 1.96.0` |
| Crate edition | `2021` (`Cargo.toml`) |

Relevant prior commits (context only):

- `68747cf` startup typed exit codes
- `7de624e` live-server integration test fixtures
- `4b620a3` RC test stabilization docs
- `aaa9d3e` operational docs exit-code alignment

**This audit does not modify Rust sources.** No `cargo fix`, dependency upgrades, or refactors were applied.

---

## Build Results

Commands run:

```bash
cargo build --all-targets
cargo test --no-run
cargo report future-incompatibilities
cargo report future-incompatibilities --id <reported-id>
```

| Command | Result |
|---------|--------|
| `cargo build --all-targets` | **PASS** (exit 0) |
| `cargo test --no-run` | **PASS** (exit 0; all test binaries linked) |
| Future-incompat report | **Present** — `sqlx-postgres v0.7.4` only |

Raw volume note: `cargo build --all-targets` prints many duplicated warning lines because the same sites are re-emitted for lib / bin / integration-test targets. Classification below uses **unique (file, line, lint, message)** sites from `--message-format=json`.

| Metric | Count |
|--------|-------|
| Unique first-party warning sites (`--all-targets`) | **201** |
| Unique sites on `evident-ledger` **lib** only | **10** (7 app + 3 vendor) |
| Sites that appear **only** on `evident-ledger` **bin** | **~145** |
| `cargo fix` suggestions advertised for lib | 3 |
| `cargo fix` suggestions advertised for `evident-ledger` bin | 14 |
| `cargo fix` suggestions advertised for `evident` bin | 2 |

### Structural amplifier (important)

The server binary (`src/main.rs`) compiles application modules via `mod …` rather than depending solely on the `evident_ledger` library. Items that are live through the **library** API are often reported as `dead_code` when the **binary** target is compiled in isolation. That inflates the all-targets warning count without meaning those paths are unused in production via the lib.

---

## Warning Summary

| Category | Count (unique sites) | Status |
|----------|----------------------|--------|
| A. Safe cleanup | **49** | Optional hygiene; not release-blocking |
| B. Intentional / accepted | **152** | Backlog / accepted for RC |
| C. Dependency | **1 crate** (`sqlx-postgres 0.7.4`, multiple internal sites) | Soft future-compat risk |
| D. Potential release blockers | **0 hard** | No security/correctness blockers identified for current edition 2021 build |

Lint mix (first-party unique):

| Lint | Count |
|------|-------|
| `dead_code` | 152 |
| `unused_imports` | 32 |
| `unused_mut` | 9 |
| `unused_variables` | 8 |

---

## Detailed Findings

### A. Safe cleanup candidates

Allowed post-audit cleanup scope (imports / unused bindings / `mut` only). **Do not** change function bodies, conditions, asserts, SQL, or API behavior.

#### Library / production sources (high value, small set)

| Location | Description | Category | Recommendation |
|----------|-------------|----------|----------------|
| `src/api/dashboard.rs:7` | unused import `post` | A | Remove import |
| `src/api/public_verify.rs:24` | unused import `PublicVerificationRateLimitAction` | A | Remove import |
| `src/api/v1/proof_state.rs:11` | unused import `TsaVerificationStatus` | A | Remove import |
| `src/api/v1/idempotency/mod.rs:7–9` | unused re-exports/imports (`AccountId`, idempotency repos/traits) | A | Trim unused imports only |
| `src/auth/mod.rs:8` | unused import `web_auth_router` | A | Remove import |
| `src/public_proof.rs:263` | unused imports `Duration`, `TimeZone` (test module) | A | Remove imports |
| `src/tsa/error_classify.rs:11` | unused import `TsaProvider` | A | Remove import |
| `src/tsa/lib.rs:16–31` | unused re-exports/imports of TSA helpers | A | Trim unused imports; do not delete modules |
| `src/paddle/mod.rs:9–13` | unused re-exports/imports | A | Trim unused imports |
| `src/middleware/public_rate_limit.rs:216` | `mut` not needed (test) | A | `let mut` → `let` |
| `src/bin/evident.rs:16` | unused import `serde_json::json` | A | Remove import |
| `src/bin/evident.rs:1127` | unused variable `tsa_serial` | A | Prefix `_tsa_serial` or remove binding |

#### Integration tests (safe, noisy)

| Location | Description | Recommendation |
|----------|-------------|----------------|
| `tests/dashboard_api.rs`, `dashboard_ui.rs`, `web_auth.rs` | unused `AppConfig` / `Arc` imports | Remove imports |
| `tests/paddle_linking.rs:15` | unused `Arc` | Remove import |
| `tests/v1_public_verify.rs:6` | unused `json` | Remove import |
| `tests/v1_public_hardening.rs:12` | unused rate-limit imports | Remove imports |
| `tests/v1_public_disclosure.rs` | unused `signer` / `config` bindings | `_signer` / `_config` |
| `tests/v1_public_hardening.rs:248` | unused `pool` | `_pool` |
| `tests/v1_public_proof_wire.rs:196` | unused `event_id` | `_event_id` |
| `tests/accounts_api.rs`, `identity_registration.rs`, `legacy_events_signature_persist.rs`, `v1_public_hardening.rs`, `v1_public_rate_limit.rs` | unnecessary `mut` | Drop `mut` |

**Autofix policy reminder:** if a future `cargo fix` touches >15 files **or** any production logic beyond imports/`_`/`mut`, stop and review the full diff before commit.

---

### B. Intentional / accepted warnings

Primarily `dead_code` retained for compatibility, alternate backends, CLI/FS helpers, serde/sqlx model fields, error enum payloads, and test fixture helpers.

#### By area (unique `dead_code` sites)

| Area | Count | Notes |
|------|------:|-------|
| `src/service/*` | 44 | Error tuple fields, unused associated helpers, entitlement/identity surfaces |
| `src/tsa/*` | 18 | Stub/job-store/writer/attest helpers retained beside live worker path |
| `src/api/*` | 16 | Alternate proof/idempotency helpers; error variants |
| `src/hash_attestation_pdf.rs` | 12 | PDF layout constants/helpers (legacy attestation PDF path) |
| `src/paddle/*` | 10 | Mock client / linking helpers / row fields |
| `tests/*` | 10 | Shared `tests/common` helpers unused in some binaries; fixture fields |
| `src/hash_attestation.rs` | 7 | Legacy hash-attestation document builders |
| `src/models/*` | 7 | FromRow/serde fields unread in some compile units |
| `src/bin/evident.rs` | 6 | CLI structs unused in current command surface |
| `src/proof_format.rs` | 5 | Legacy format constants/helpers |
| `vendor/notary-pdf` | 2 | Internal PDF helpers |
| `vendor/notary-tsa` | 1 | `build_tsq` / `path_to_str` helpers |
| Other (`auth`, `db`, `merkle`, `middleware`, `public_*`, `state`, `verify` bin) | ≤3 each | Small leftovers / reserved APIs |

#### Representative intentional patterns

| Pattern | Example | Why accepted |
|---------|---------|--------------|
| Error enum `field 0` unread | `src/api/account.rs:43`, many `service/*` | Payload kept for `Debug` / future mapping; not unused logic |
| Unused enum variants | `api/v1/errors::Forbidden`, ledger/`identity_*` variants | Reserved API / completeness |
| Alternate idempotency backend | `PostgresIdempotencyRepository`, in-memory trait | Scaffolding; wired selectively |
| TSA FS job store / writer | `src/tsa/job_store.rs`, `writer.rs` | Compatibility / non-default path |
| Hash attestation + PDF | `hash_attestation*.rs` | Legacy attestation surface |
| `tests/common` helpers | `setup_test_env`, `test_app_state`, … | Shared; unused in every integration binary |
| Vendor helper methods | `notary-pdf` color helpers; `notary-tsa` openssl helpers | Internal library surface |

**Recommendation:** leave as backlog. Mass `dead_code` deletion is **not** safe cleanup under this audit’s rules (would change API surface / remove compatibility paths).

---

### C. Dependency warnings

#### `sqlx-postgres v0.7.4` — future incompatibility

| Field | Value |
|-------|-------|
| Crate | `sqlx-postgres` |
| Version | `0.7.4` (via workspace `sqlx = "0.7"`) |
| Affects current build? | **No** — build/test-compile succeed on edition 2021 / rustc 1.96 |
| Lint theme | never-type fallback (`!: Decode`) will fail under **Rust 2024** / future rustc |
| Upstream sites (crate-internal) | `connection/executor.rs` (`prepare`), `copy.rs` (`abort`, `finish`, `pg_begin_copy_out`) |
| Newer versions available | `0.8.x`, `0.9.0-alpha.1` (per `cargo report`) |
| Risk on Rust upgrade | Soft → hard error if toolchain/edition advances without sqlx bump |
| Security impact of warning itself | None observed |

Full report source: `cargo report future-incompatibilities` (package-focused detail via `--package sqlx-postgres@0.7.4`).

**Recommendation:** track **sqlx 0.8+ upgrade** as a dedicated change (not drive-by). Out of scope for warning-noise cleanup.

No other dependency future-incompat packages were reported in this baseline.

---

### D. Potential release blockers

| Concern | Finding |
|---------|---------|
| Security-relevant warnings | **None** identified in this pass |
| Correctness / production-behavior warnings | **None** — remaining warnings are unused code/imports, not behavioral diagnostics |
| Future Rust compatibility | **Soft:** `sqlx-postgres 0.7.4` never-type fallback; not blocking current RC toolchain |
| Hard release blocker for Stage 13.7 RC stable cut? | **No** |

---

## Lib-only baseline (production library signal)

Unique warnings when building `--lib` (clearest production signal):

| Location | Lint | Notes |
|----------|------|-------|
| `src/api/dashboard.rs:7` | unused_imports | Safe |
| `src/api/public_verify.rs:24` | unused_imports | Safe |
| `src/api/v1/proof_state.rs:11` | unused_imports | Safe |
| `src/api/account.rs:43` | dead_code (`field 0`) | Intentional error payload |
| `src/api/v1/idempotency/postgres.rs:94` | dead_code (`pool`) | Scaffolding |
| `src/service/billing.rs:23` | dead_code (`email`) | Row field unread |
| `src/tsa/attest.rs:36` | dead_code (`tsr_content_hash`) | Helper retained |
| `vendor/notary-pdf` (2) | dead_code | Vendor |
| `vendor/notary-tsa` (1) | dead_code | Vendor |

---

## Decision

### Fix before stable

**Required:** none (no hard blockers).

**Optional hygiene (recommended if a small cleanup PR is desired):**

1. Remove unused imports listed under **A** for `src/` (especially the 3 lib-suggested fixes).
2. Optionally tidy test unused imports / `_` bindings / `mut` (same category A).
3. Do **not** run unconstrained workspace `cargo fix` without reviewing the diff; stop if >15 files or any logic change.

### Accepted / backlog

1. Mass `dead_code` across service/TSA/hash-attestation/idempotency/models (**B**).
2. Bin-target warning inflation from `mod`-embedded server binary (architectural; separate refactor if ever pursued).
3. `sqlx` 0.7 → 0.8+ migration for future-incompat (**C**) as its own tracked work item.
4. Vendor `notary-pdf` / `notary-tsa` dead helpers.

---

## Outcome

Transparent RC picture:

- **~49** warnings are safe, mechanical cleanup candidates.
- **~152** are intentional/backlog `dead_code` (or structural bin noise).
- **1** dependency future-incompat (`sqlx-postgres 0.7.4`) — soft, not current-build breaking.
- **0** hard release blockers for security/correctness on the audited baseline.

Stable release is **not blocked** by this warning set; cleanup is optional hygiene, and sqlx upgrade remains a separate compatibility track.
