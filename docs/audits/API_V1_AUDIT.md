# API v1 Audit

Date: 2026-07-17

Scope:

- `src/api/`
- `src/auth/`
- `src/service/`
- `src/client.rs`
- `migrations/`

Note: `src/account/` **does not exist** — account handlers live in `src/api/account.rs`.

Framework: **Axum** (`Router`, `nest`, handlers under `src/api/`, mounted from `src/main.rs`).

---

## Audit Context

This audit reflects the current repository state after v1 API implementation
work has already been completed.

The original purpose of the audit was pre-implementation mapping, but the
repository already contains implemented v1 routes and related infrastructure.

This document therefore serves as a current-state verification and contract
mapping audit rather than a pre-development inventory.

A prior draft of this file dated 2026-07-16 captured the pre-v1 state (no `/v1`
router). That snapshot is superseded by this post-implementation verification.

---

## Raw findings

### Commands (2026-07-17)

```text
$ find src/api -type f | sort
src/api/account.rs
src/api/backup.rs
src/api/chains.rs
src/api/events.rs
src/api/identity.rs
src/api/mod.rs
src/api/verify.rs
src/api/v1/account.rs
src/api/v1/auth.rs
src/api/v1/errors.rs
src/api/v1/event_access.rs
src/api/v1/events.rs
src/api/v1/idempotency/{canonical,mod,model,postgres,repository}.rs
src/api/v1/mod.rs
src/api/v1/proof.rs
src/api/v1/proof_material.rs
src/api/v1/proof_status.rs
src/api/v1/submit_event.rs
src/api/v1/validation.rs
src/api/v1/verify.rs

$ find src/api -type d | sort
src/api
src/api/v1
src/api/v1/idempotency

$ rg -n 'nest' src/main.rs src/api/v1/mod.rs
src/main.rs:70:        .nest("/account", api::account::router(...))
src/main.rs:71:        .nest("/backup", api::backup::router(...))
src/main.rs:72:        .nest("/chains", api::chains::router(...))
src/main.rs:73:        .nest("/events", api::events::router(...))
src/main.rs:74:        .nest("/verify", api::verify::router(...))
src/main.rs:75:        .nest("/identity", api::identity::router(...))
src/main.rs:76:        .nest("/v1", api::v1::router(...))
src/api/v1/mod.rs:22-25: .nest("/events|/proof|/verify|/account", ...)

$ rg -n '"/v1' src/
src/main.rs:76:        .nest("/v1", api::v1::router(state.clone()));
```

**Versioning:** `/v1` router **exists** at `src/api/v1/mod.rs`, mounted from `src/main.rs`.
Legacy unversioned routes remain mounted in parallel.

### v1 route table (implemented handlers)

| Full path | Method | File / handler | Auth |
| --------- | ------ | -------------- | ---- |
| `/v1/events` | POST | `v1/events.rs` → `submit_v1_event` | `V1Auth` |
| `/v1/proof/:event_id` | GET | `v1/proof.rs` → `build_proof_response` | `V1Auth` + ownership |
| `/v1/verify/:event_id` | GET | `v1/verify.rs` → stub (`{event_id}`) | `V1Auth` + ownership |
| `/v1/account/capabilities` | GET | `v1/account.rs` → `501 NotImplemented` | `V1Auth` |

v1 router applies `request_id_layer` middleware (`v1/mod.rs`).

### Legacy / pre-v1 routes (still mounted)

| Full path | Method | File | Auth |
| --------- | ------ | ---- | ---- |
| `/events` | POST | `events.rs` → `ledger::submit_event` | `AuthedAccount` |
| `/verify/:chain_id` | GET | `verify.rs` | **none** |
| `/verify/proof/:chain_id` | GET | `verify.rs` | **none** |
| `/verify/hash` | POST | `verify.rs` | **none** |
| `/verify/:chain_id/attestation` | GET | `verify.rs` | **none** |
| `/verify/:chain_id/attestation.pdf` | GET | `verify.rs` | **none** |
| `/verify/hash/:hash/attestation.pdf` | GET | `verify.rs` | **none** |
| `/account/capabilities` | GET | `account.rs` | `AuthedAccount` |
| `/account/usage` | GET | `account.rs` | `AuthedAccount` |
| `/account/key-status` | GET | `account.rs` | `AuthedAccount` |
| `/account/dev/change-plan` | POST | `account.rs` | `AuthedAccount` |
| `/backup/create` | POST | `backup.rs` | `AuthedAccount` |
| `/backup/list` | GET | `backup.rs` | `AuthedAccount` |
| `/backup/:id` | GET | `backup.rs` | `AuthedAccount` |
| `/backup/:id/download` | GET | `backup.rs` | `AuthedAccount` |
| `/chains` | POST | `chains.rs` | `AuthedAccount` |
| `/identity` | GET | `identity.rs` | **none** |
| `/`, `/verify-ui`, `/whitepaper`, `/whitepaper.pdf` | GET | `main.rs` static | **none** |

---

## Contract Mapping

| API Contract (`docs/API.md`) | Current route | File / handler | Status |
| ---------------------------- | ------------- | -------------- | ------ |
| `POST /v1/events` | `POST /v1/events` | `v1/events.rs` → `submit_v1_event` | **matches** (see minor notes) |
| `GET /v1/proof/{event_id}` | `GET /v1/proof/:event_id` | `v1/proof.rs` → `proof_material::build_proof_response` | **matches** (see notes) |
| `GET /v1/verify/{event_id}` | `GET /v1/verify/:event_id` | `v1/verify.rs` stub | **exists-diverged** |
| `GET /v1/account/capabilities` | `GET /v1/account/capabilities` | `v1/account.rs` → `501` | **exists-diverged** |
| `GET /v1/backup/*` | — | — | **missing** |

### Divergence / match notes

#### `POST /v1/events` — **matches** (`docs/API.md` §4, post Stage 3 sync)

Implemented: required `Idempotency-Key` header, `event_type` enum, `request_id` in response,
derived `proof_status`, `trust_level`, unified v1 error envelope, ownership via chain access
(`404 not_found`).

Minor residual:

- Legacy `POST /events` still mounted; writes same `events` table with `signature = ""`.
- TSA stamp still uses full-chain root post-commit (not prefix root) — TSA field behavior
  documented in §6.

#### `GET /v1/proof/{event_id}` — **matches** (`docs/API.md` §6)

Implemented: prefix snapshot semantics (`sequence <= target`), ownership guard, persisted
signature read-path (`build_proof_snapshot_read` + `verify_root`), pending/anchored envelopes,
`request_id`, `tsa: null` when absent.

Minor residual:

- Legacy events (`signature = ""` via `POST /events`) → v1 proof returns `pending`.
- Invalid persisted signature → `pending` (not `failed`); `failure_signal` policy not yet
  implemented (`docs/API_IMPLEMENTATION_PLAN.md` Stage 4 §3).

#### `GET /v1/verify/{event_id}` — **exists-diverged** (`docs/API.md` §7)

Route and ownership exist; handler returns stub JSON `{ "event_id": "..." }` only.
No chain/file verification, no `?file_hash=`, no `409 proof_not_ready` behavior.

#### `GET /v1/account/capabilities` — **exists-diverged**

Route registered; returns `501 not_implemented`. Working capabilities live at legacy
`GET /account/capabilities` (unversioned, different error shape).

#### `GET /v1/backup/*` — **missing**

Placeholder in contract only. Legacy `/backup/*` routes exist at unversioned paths;
no `/v1/backup/*` handlers are registered.

---

## Legacy Endpoints

Endpoints outside the v1 contract (discovered in code, not assumed):

| Found route | Handler | Notes |
| ----------- | ------- | ----- |
| `POST /events` | `events.rs` | Unversioned; shares `events` table with v1; old error format |
| `GET/POST /verify/*` | `verify.rs` | Unversioned; chain-scoped; no auth; old error format |
| `GET /account/*` | `account.rs` | Unversioned; capabilities logic exists; old error format |
| `POST/GET /backup/*` | `backup.rs` | Unversioned; no v1 contract equivalent |
| `POST /chains` | `chains.rs` | Unversioned; not in v1 contract |
| `GET /identity` | `identity.rs` | Unversioned; public; used by CLI/GUI |
| Static HTML/PDF | `main.rs` | Unversioned; static assets |

### Legacy vs v1 coexistence (factual state)

- Legacy routes remain mounted alongside `/v1` in `src/main.rs`.
- Legacy handlers return errors as `{ "error": "<plain string>" }` (see Error Format Audit).
- The v1 error envelope (`{ "error": { "code", "message", "request_id" } }`) applies only to
  routes under the `/v1` router.
- No code path automatically redirects or migrates legacy requests to v1 handlers.

---

## Auth Audit

Note: `src/auth/` **does not exist** — auth logic is in `src/auth.rs` (no auth middleware module).

### Findings

| Component | Location | Behavior |
| --------- | -------- | -------- |
| API key resolution | `src/auth.rs` `AuthedAccount` | SHA-256 hash → `api_keys` lookup |
| v1 auth wrapper | `src/api/v1/auth.rs` `V1Auth` | Maps auth failure → `ApiError::Unauthorized` (v1 envelope) |
| Legacy auth rejection | `src/auth.rs` `AuthError` | `401` with `{ "error": "<plain string>" }` — **not** v1 envelope |

**Middleware:** No global auth middleware. Per-handler `FromRequestParts` extractors
(`AuthedAccount` / `V1Auth`).

**Ownership (event-level):**

| Endpoint | Implementation | Status |
| -------- | -------------- | ------ |
| `GET /v1/proof/{event_id}` | `event_access::verify_event_access` | **implemented** — `404 not_found` |
| `GET /v1/verify/{event_id}` | same (stub handler) | **implemented** |
| `POST /v1/events` | `ensure_chain_access_in_tx` | **implemented** — chain ownership |
| Legacy `/verify/*` | none | **no ownership** |

Chain ownership on legacy `POST /events` exists via `ledger::ensure_chain_access_in_tx`.

---

## Error Format Audit

### v1 (`src/api/v1/errors.rs`) — **matches contract** (`docs/API.md` §2)

```json
{ "error": { "code": "...", "message": "...", "request_id": "..." } }
```

- `ApiError` enum with lowercase snake_case codes
- `request_id_layer` middleware on v1 router
- Unit test for unauthorized envelope shape

### Legacy — **diverged** (plain string or partial JSON)

| Location | Format |
| -------- | ------ |
| `src/auth.rs` `AuthError` | `{ "error": "<string>" }` |
| `src/api/verify.rs` `ApiError` | `{ "error": "<string>" }` |
| `src/api/account.rs` `DevAccountApiError` | `{ "error": "<string>" }` |
| `src/service/ledger.rs` `LedgerError` | `{ "error": "<string>" }` |
| `src/service/backup.rs` `BackupError` | `{ "error": "<string>" }` |

**Current split:** only routes under `/v1` emit the contract error envelope. Legacy
`/events`, `/verify/*`, `/account/*`, and `/backup/*` continue to use the old format.
There is no shared error module bridging legacy handlers to the v1 envelope.

Domain codes reserved in §7 but not yet in `ApiError`: `proof_not_ready`, `proof_generation_failed`.

---

## Idempotency Audit

### v1 (implemented)

| Piece | Location |
| ----- | -------- |
| Table | `migrations/20260717000000_idempotency_records.sql` → `idempotency_records` |
| Header | `Idempotency-Key` in `v1/events.rs` |
| Canonical hash | `v1/idempotency/canonical.rs` |
| Repository | `v1/idempotency/{model,repository,postgres}.rs` |
| Integration test | `tests/v1_events_idempotency.rs` |

Scoped per `(account_id, idempotency_key)` with 24h TTL; replay 200 / conflict 409.

### Legacy (still active)

| Piece | Location |
| ----- | -------- |
| Body field | `idempotency_key` in `SubmitEventRequest` |
| DB constraint | `uniq_idem UNIQUE (chain_id, idempotency_key)` on `events` |
| Logic | `ledger::submit_event` pre-insert lookup |

Both systems coexist; v1 path does not use legacy `(chain_id, idempotency_key)` dedup.

---

## Summary

### Endpoints ready for /v1 with minimal remaining work

| Endpoint | Notes |
| -------- | ----- |
| `POST /v1/events` | Production-ready for Stage 2 scope |
| `GET /v1/proof/{event_id}` | Production-ready for Stage 2/C + Stage 4 §2 persistence |

### Endpoints requiring substantial new logic (not just path)

| Endpoint | Gap |
| -------- | --- |
| `GET /v1/verify/{event_id}` | Full §7 implementation: chain/file split, prefix verify, pending 409 |
| `GET /v1/account/capabilities` | Wire to existing `get_account_capabilities` + v1 envelope |

### Endpoints missing entirely

| Endpoint | Notes |
| -------- | ----- |
| `GET /v1/backup/*` | Contract placeholder only |

### Legacy endpoints

Legacy routes remain mounted in `src/main.rs`. v1 routes run in parallel under `/v1`.
Legacy `/events` and `/verify/*` have no deprecation markers in code.

### Stage 1 (auth + error contract) — current status

| Deliverable | Status |
| ----------- | ------ |
| `/v1` router skeleton | **Present** |
| v1 error envelope + `request_id` | **Present** (v1 router only) |
| v1 auth via `X-API-KEY` | **Present** (`V1Auth` extractor on v1 handlers) |
| Event-level ownership (proof/verify) | **Present** |
| Legacy routes on v1 envelope | **Absent** — legacy handlers unchanged |

### Remaining contract gaps (current state vs `docs/API.md`)

- `GET /v1/verify/{event_id}` — stub handler; full §7 behavior not implemented
- `GET /v1/account/capabilities` — returns `501`; legacy `/account/capabilities` serves data
- `GET /v1/backup/*` — not implemented
- Stage 4 §3 (`failure_signal` policy) — not implemented
- Signature persistence and `UNIQUE(chain_id, sequence)` — present (commits `bb43af7`, `adf6ad3`)

### Implementation History Note

The following v1 components were already present during this audit:

- `POST /v1/events`
- `GET /v1/proof/{event_id}`
- v1 authentication middleware
- `idempotency_records` storage
- derived `proof_status` flow

This audit does not verify the original implementation process history;
it verifies the current repository state against the public API contract.

Note: v1 auth is implemented as `V1Auth` extractor + `request_id_layer` on the v1 router
(not a global middleware module; see Auth Audit).

**Git history visible in this repository** (`git log --oneline --all`):

| Path / component | Commits touching it |
| ---------------- | ------------------- |
| `src/api/v1/` (entire tree) | `6648656`, `adf6ad3`, `bb43af7` |
| `src/api/v1/idempotency/` + `migrations/20260717000000_idempotency_records.sql` | `6648656` |
| `src/auth.rs` (legacy `AuthedAccount`; predates v1) | `2f63183`, `ad4da32` |

Commit mapping (where unambiguous):

- `6648656` — initial v1 module: router mount, auth/errors, events, proof, idempotency,
  verify/account stubs
- `adf6ad3` — `UNIQUE(chain_id, sequence)` constraint + v1 conflict mapping
- `bb43af7` — event signature persistence for v1 proof read-path

v1 work appears as **multiple sequential commits**, not a single monolithic commit.
The 2026-07-16 pre-v1 audit draft in this file's git history predates `6648656`.
