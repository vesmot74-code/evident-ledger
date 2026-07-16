# API v1 Audit

Date: 2026-07-16

Scope:

- `src/api/`
- `src/auth/`
- `src/service/`
- `src/client.rs`
- `migrations/`

Framework: **Axum** (`Router`, `nest`, route handlers in `src/api/*`, mounted from `src/main.rs`).

---

## Raw findings

### Route inventory (`grep` / file read)

| File | Line | Route (full path) | Handler | Auth | Purpose |
| ---- | ---- | ----------------- | ------- | ---- | ------- |
| `src/main.rs` | 70 | `GET /account/*` | nested router | varies | Account sub-routes |
| `src/main.rs` | 71 | `POST/GET /backup/*` | nested router | `AuthedAccount` | Server backup CRUD |
| `src/main.rs` | 72 | `POST /chains` | `chains::handler` | `AuthedAccount` | Create new chain UUID |
| `src/main.rs` | 73 | `POST /events` | `events::handler` | `AuthedAccount` | Submit ledger event |
| `src/main.rs` | 74 | `GET/POST /verify/*` | nested router | **none** | Chain verify, proof export, attestations |
| `src/main.rs` | 75 | `GET /identity` | `identity::get_identity` | **none** | Server signing public key |
| `src/api/account.rs` | 20–23 | `GET /account/usage` | `usage_handler` | `AuthedAccount` | Monthly usage counters |
| `src/api/account.rs` | 21 | `GET /account/capabilities` | `capabilities_handler` | `AuthedAccount` | Tariff / entitlements |
| `src/api/account.rs` | 22 | `GET /account/key-status` | `key_status_handler` | `AuthedAccount` | API key metadata |
| `src/api/account.rs` | 23 | `POST /account/dev/change-plan` | `dev_change_plan_handler` | `AuthedAccount` | Dev-only plan switch |
| `src/api/backup.rs` | 26 | `POST /backup/create` | `create_handler` | `AuthedAccount` | Create chain backup |
| `src/api/backup.rs` | 27 | `GET /backup/list` | `list_handler` | `AuthedAccount` | List backups |
| `src/api/backup.rs` | 28 | `GET /backup/:backup_id/download` | `download_handler` | `AuthedAccount` | Download backup JSON |
| `src/api/backup.rs` | 29 | `GET /backup/:backup_id` | `info_handler` | `AuthedAccount` | Backup metadata |
| `src/api/chains.rs` | 7 | `POST /chains` | `handler` | `AuthedAccount` | Allocate chain |
| `src/api/events.rs` | 12 | `POST /events` | `handler` | `AuthedAccount` | Submit event |
| `src/api/verify.rs` | 37 | `GET /verify/:chain_id` | `handler_verify` | **none** | Chain-wide verification |
| `src/api/verify.rs` | 41 | `GET /verify/proof/:chain_id` | `handler_proof` | **none** | Export chain proof JSON |
| `src/api/verify.rs` | 46 | `POST /verify/hash` | `handler_verify_hash` | **none** | Lookup events by file hash |
| `src/api/verify.rs` | 51 | `GET /verify/:chain_id/attestation` | `handler_attestation` | **none** | SAC attestation document |
| `src/api/verify.rs` | 56 | `GET /verify/:chain_id/attestation.pdf` | `handler_attestation_pdf` | **none** | SAC PDF |
| `src/api/verify.rs` | 61–63 | `GET /verify/hash/:hash/attestation.pdf` | `handler_hash_attestation_pdf` | **none** | Hash attestation PDF |
| `src/api/identity.rs` | 14 | `GET /identity` | `get_identity` | **none** | Ed25519 public key |

**No `/v1` prefix anywhere.** No v1 router module exists.

### Related non-contract routes (informational)

These exist but are **not** in `docs/API.md` v1 contract:

- Static/HTML: `/`, `/verify-ui`, `/whitepaper`, `/whitepaper.pdf`
- `POST /chains` — chain creation helper
- Attestation/PDF sub-routes under `/verify/*`
- `GET /identity` — used by CLI/GUI for TOFU key pinning

---

## Contract Mapping

| API Contract (`docs/API.md`) | Current route | File / handler | Status |
| ---------------------------- | ------------- | -------------- | ------ |
| `POST /v1/events` | `POST /events` | `src/api/events.rs` → `handler` → `service/ledger::submit_event` | **exists-diverged** |
| `GET /v1/proof/{event_id}` | `GET /verify/proof/{chain_id}` | `src/api/verify.rs` → `handler_proof` → `service/verification::export_proof` | **exists-diverged** |
| `GET /v1/verify/{event_id}` | `GET /verify/{chain_id}` | `src/api/verify.rs` → `handler_verify` → `service/verification::verify_chain` | **exists-diverged** |
| `GET /v1/account/capabilities` | `GET /account/capabilities` | `src/api/account.rs` → `capabilities_handler` | **exists-unversioned** |
| `GET /v1/backup/*` | `POST /backup/create`, `GET /backup/list`, `GET /backup/:id`, `GET /backup/:id/download` | `src/api/backup.rs` | **exists-diverged** |

### Divergence details

#### `POST /v1/events` (docs/API.md §4)

| Contract | Current |
| -------- | ------- |
| Path `/v1/events` | `/events` |
| Optional `Idempotency-Key` **header**, scoped per `account_id` | Required `idempotency_key` in **JSON body**, scoped per `(chain_id, idempotency_key)` |
| Request: `chain_id`, `file_hash`, `event_type` | Request: `chain_id`, `file_hash`, `idempotency_key`, optional `parent_event_id` — no `event_type` |
| Response: `event_id`, `chain_id`, `sequence`, `proof_status`, `trust_level` | Response: nested `proof`, `events[]`, `tsa`, `head_event_id`, `cached` — no `proof_status` or `trust_level` |
| Unified error envelope | `{ "error": "<plain string>" }` via `LedgerError` |
| Idempotency conflict → 409 `CONFLICT` | Duplicate DB constraint → 409 `"Duplicate idempotency key"` (string, no code) |
| Replay returns stored response | Replay returns `{ event_id, chain_id, head_event_id, cached: true }` (subset, not full stored body) |

#### `GET /v1/proof/{event_id}` (docs/API.md §6)

| Contract | Current |
| -------- | ------- |
| Path param `event_id` | Path param `chain_id` |
| Single-event proof artifact (flat schema) | Chain-level export with nested `proof` + `events[]` array |
| Required fields: `proof_version`, `proof_type`, `leaf_version`, `event_id`, `chain_id`, `sequence`, `parent_event_id`, `file_hash`, `merkle_root`, `signature`, `public_key`, `tsa`, `created_at` | Uses `proof.version`, `proof.root`, `head_event_id`; per-event fields inside `events[]`; no top-level `merkle_root`/`signature` per event |
| Requires auth + ownership | **No authentication** on handler |
| 403 for cross-account access | Not implemented (public by chain UUID) |

#### `GET /v1/verify/{event_id}` (docs/API.md §7)

| Contract | Current |
| -------- | ------- |
| Path param `event_id` | Path param `chain_id` |
| Query `?file_hash=<hex>` only | No `file_hash` query; separate `POST /verify/hash` for hash lookup |
| Response: `{ chain: { valid, merkle_valid, signature_valid, errors }, file: { status } }` | Response: `{ chain_id, valid, blocks, errors, head_event_id, proof }` — combined `valid`, no `file.status` |
| `proof_status` gating (409/422) | No `proof_status` concept in code or DB |
| Requires auth + ownership before proof_status | **No authentication** |
| Ownership before proof_status check | N/A — neither check exists |

#### `GET /v1/account/capabilities` (docs/API.md §8)

| Contract | Current |
| -------- | ------- |
| Path `/v1/account/capabilities` | `/account/capabilities` |
| Errors use unified envelope | Handler returns `String` errors on DB failure; auth uses legacy `{ "error": "string" }` |
| Capability fields per SYSTEM_CONTRACT §13 | Core fields present via `AccountCapabilities`; handler adds extra `account_id`, `dev_tools_available` |

#### `GET /v1/backup/*` (docs/API.md §9)

| Contract | Current |
| -------- | ------- |
| Placeholder — out of scope for v0.1-draft | Four concrete authenticated routes under `/backup/*` |
| Not part of Public API v1 | Internal/product backup API exists |

---

## Auth Audit

### API Key flow

**Present:** `X-API-KEY` → SHA-256 hash → `api_keys` lookup → `AuthedAccount { account_id, key_hash }`.

Implementation: `src/auth.rs` — `AuthedAccount` implements Axum `FromRequestParts` (extractor pattern, not a separate middleware layer). Handlers declare `auth: AuthedAccount` as a parameter; rejection returns `AuthError`.

```
X-API-KEY
    |
    v
SHA-256 → key_hash
    |
    v
SELECT account_id FROM api_keys WHERE key_hash = $1 AND revoked_at IS NULL
    |
    v
AuthedAccount { account_id, key_hash }
    |
    v
Handler (events, account, backup, chains)
```

**Not authenticated today:** all routes in `src/api/verify.rs`, `src/api/identity.rs`.

### Error behavior (invalid API key)

| Condition | HTTP | Body |
| --------- | ---- | ---- |
| Missing header | 401 | `{ "error": "Missing X-API-KEY header" }` |
| Invalid/revoked key | 401 | `{ "error": "Invalid or revoked API key" }` |

Contract requirement (docs/API.md §2): `{ "error": { "code": "UNAUTHORIZED", "message": "...", "request_id": "..." } }`.

**Gap:** flat string `error`, no `code`, no `request_id`.

### Ownership

#### Chain-level (exists)

`src/service/ledger.rs` (`submit_event`):

- On `POST /events`, checks `chains.account_id` against authenticated `account_id`.
- Returns 403 `"Chain belongs to a different account"` if mismatch.
- First commit on unowned chain assigns `account_id`.

`src/service/backup.rs`: backup operations scoped by `account_id` in queries.

#### Event-level (missing for proof/verify)

No code path checks:

```
event_id → events.chain_id → chains.account_id == AuthedAccount.account_id
```

for proof or verify endpoints. Verify/proof handlers load data by `chain_id` without auth.

**Conclusion:** Ownership verification for `GET /v1/proof/{event_id}` and `GET /v1/verify/{event_id}` **must be implemented during API v1 migration** (docs/API.md §1, API_IMPLEMENTATION_PLAN.md Stage 4).

---

## Error Format Audit

### Current error types

| Type | File | HTTP statuses | Response shape |
| ---- | ---- | ------------- | -------------- |
| `AuthError` | `src/auth.rs` | 401 | `{ "error": "<string>" }` |
| `LedgerError` | `src/service/ledger.rs` | 403, 404, 409, 429, 503, 500 | `{ "error": "<string>" }` |
| `ApiError` | `src/api/verify.rs` | 400, 404, 500 | `{ "error": "<string>" }` |
| `BackupError` | `src/service/backup.rs` | 403, 404, 500 | `{ "error": "<string>" }` or `{ "error": "not_found" }` |
| `DevAccountApiError` | `src/api/account.rs` | 400, 403, 404, 500 | `{ "error": "<string>" }` |
| Plain `String` | `src/api/chains.rs`, `src/api/account.rs` (usage/capabilities/key-status) | 500 (Axum default mapping) | unstructured |

### Contract gaps

1. **No unified error enum** for Public API v1.
2. **No machine-readable `code`** field (e.g. `UNAUTHORIZED`, `FORBIDDEN`, `CONFLICT`).
3. **No `request_id`** generation or propagation.
4. **No shared `{ error: { code, message, request_id } }` wrapper.**

`request_id` appears only in hash attestation documents (`src/hash_attestation.rs`), not in HTTP API errors.

### Files likely requiring changes for v1 error envelope (Stage 1)

| File | Reason |
| ---- | ------ |
| `src/auth.rs` | 401 responses |
| `src/service/ledger.rs` | Event submission errors |
| `src/api/verify.rs` | Verify/proof errors (new v1 handlers) |
| `src/api/account.rs` | Capabilities + dev endpoint errors |
| `src/service/backup.rs` | Backup errors (if exposed under v1 later) |
| `src/api/chains.rs` | Currently returns raw `String` |
| New module (e.g. `src/api/error.rs`) | Shared envelope, codes, request_id middleware |

---

## Idempotency Audit

### Current implementation

| Aspect | Status | Location |
| ------ | ------ | -------- |
| `Idempotency-Key` HTTP header | **Absent** | — |
| Body field `idempotency_key` | **Present** (required) | `src/models/event.rs`, `SubmitEventRequest` |
| DB storage | **Partial** — key stored on `events` row | `migrations/20260628192818_init.sql` |
| Unique constraint | `(chain_id, idempotency_key)` | `uniq_idem` |
| Scoped per `account_id` | **No** — scoped per `chain_id` | `src/service/ledger.rs:129` |
| `request_hash` | **Absent** | — |
| `idempotency_records` table | **Absent** | — |
| TTL / expiration | **Absent** | — |
| Replay protection | **Partial** — returns cached `{ event_id, cached: true }` if same chain+key | `ledger.rs:124–143` |
| Conflict on different body | **No** — same key+different body hits DB unique constraint → generic CONFLICT | Not body-aware |
| Response replay (full stored body) | **No** | — |

### Client usage

`src/client.rs`, `src/product.rs`: generate UUID, send as JSON `idempotency_key` field (not header).

### Reuse assessment

Existing `events.idempotency_key` column and `(chain_id, idempotency_key)` lookup **cannot** be reused as-is for v1 contract:

- Contract requires header-based, account-scoped, body-hash-aware deduplication with 24h TTL and full response replay.
- Requires new `idempotency_records` table per API_IMPLEMENTATION_PLAN.md Stage 2.

---

## Database Model Audit

### Migration files

| File | Purpose |
| ---- | ------- |
| `20260628192818_init.sql` | `chains`, `events` |
| `20260628192900_tsa_tokens.sql` | `tsa_tokens` |
| `20260628202402_add_sequence_to_events.sql` | `events.sequence` |
| `20260628202432_add_sequence_to_events.sql` | sequence follow-up |
| `20260706093531_add_file_hash_index.sql` | index on `events.file_hash` |
| `20260715120000_accounts_and_api_keys.sql` | `accounts`, `api_keys`, `chains.account_id` |
| `20260715220000_backups.sql` | `backups` |
| `20260716000000_tariff_plans_and_billing.sql` | `tariff_plans`, `usage_monthly`, plan FK on accounts |

### Tables by domain

| Domain | Tables | Notes |
| ------ | ------ | ----- |
| Accounts | `accounts`, `api_keys` | Auth resolution |
| Events / chains | `chains`, `events` | Core ledger; `chains.account_id` nullable for legacy data |
| Proofs | *(inline)* | Proof generated at commit time in response JSON; no `proofs` table; no `proof_status` column |
| TSA | `tsa_tokens` | Per `(chain_id, merkle_root)` |
| Backups | `backups` | Account-scoped server backups |
| Billing | `tariff_plans`, `usage_monthly` | Limits enforced in `submit_event` |
| Idempotency (v1 target) | **missing** | `idempotency_records` not present |

### `idempotency_records` readiness

No table, migration, or service module exists for v1 idempotency state. Stage 2 requires new migration (out of scope for this audit).

### `proof_status` readiness

Not stored in database. Current commit flow is synchronous: proof fields returned inline in `submit_event` response. v1 `pending`/`anchored`/`failed` lifecycle not implemented.

---

## Summary

### Ready for migration

Endpoints whose **business logic** can be reused with routing/error/response wrapping:

| Current | v1 target | Reuse notes |
| ------- | --------- | ----------- |
| `GET /account/capabilities` | `GET /v1/account/capabilities` | `get_account_capabilities()` is ready; add `/v1` mount + error envelope; decide whether to keep extra fields |
| `POST /events` core ledger logic | `POST /v1/events` | `submit_event()` handles chain ownership, usage limits, Merkle/TSA; needs new request/response schema, idempotency layer, `proof_status`/`trust_level` |
| `service/verification.rs` | v1 verify internals | Merkle recompute + structural checks exist; must be refactored to per-`event_id`, chain/file split, ownership gate |

### Requires changes

| Area | Changes needed |
| ---- | -------------- |
| All v1 routes | Add `/v1` router nest in `src/main.rs` |
| Error handling | Unified envelope + codes + `request_id` (Stage 1) |
| `POST /events` | Header idempotency, v1 request/response, error codes |
| Proof endpoint | New `GET /v1/proof/{event_id}` — event-scoped schema (API.md §6), auth, ownership |
| Verify endpoint | New `GET /v1/verify/{event_id}?file_hash=` — auth, ownership-before-proof_status, chain/file response |
| Capabilities | v1 path + contract error format for 401/403/500 |
| Backup | Do not expose under v1; existing `/backup/*` stays internal |

### Missing functionality

| Feature | Status |
| ------- | ------ |
| `/v1` route prefix | Missing entirely |
| Unified error envelope | Missing |
| `request_id` in API errors | Missing |
| Event-level ownership on proof/verify | Missing |
| `Idempotency-Key` header + `idempotency_records` | Missing |
| Dedicated canonical JSON `request_hash` | Missing (decision documented, not implemented) |
| `proof_status` enum lifecycle | Missing in DB and handlers |
| `trust_level` in event response | Missing (derived today only in CLI output) |
| `GET /v1/proof/{event_id}` per contract | Missing (chain-level `/verify/proof/{chain_id}` differs) |
| `GET /v1/verify/{event_id}` per contract | Missing (chain-level, unauthenticated, wrong shape) |
| File verification via query param | Missing (`file.status` not implemented) |

### Stage 1 readiness

| Component | Status |
| --------- | ------ |
| **Authentication** | **Partial** — `X-API-KEY` → `AuthedAccount` works for protected routes; verify/proof/identity are public; no v1 mount |
| **Error handling** | **Not ready** — multiple ad-hoc `{ "error": string }` shapes; no codes, no `request_id` |
| **Ownership** | **Partial** — chain ownership on commit exists; event-level ownership for proof/verify absent |
| **Idempotency** | **Not ready** — legacy body-field, chain-scoped; v1 header/account-scoped layer not built |

### Recommended implementation order

Per `docs/API_IMPLEMENTATION_PLAN.md`:

```
Stage 1 — Authentication + Error Contract (/v1 mount, envelope, request_id)
    ↓
Stage 2 — Idempotency Layer (idempotency_records, request_hash)
    ↓
Stage 3 — POST /v1/events (schema alignment)
    ↓
Stage 4 — GET /v1/proof + GET /v1/verify (ownership, proof_status, file.status)
    ↓
Stage 5 — GET /v1/account/capabilities alignment; backup remains out of scope
```

Legacy routes (`/events`, `/verify/*`, etc.) should remain during migration for existing CLI/GUI clients until v1 clients are cut over.
