# Finding P1 — Legacy `/events` accepts identity fields without v1 validation

Date: 2026-07-23  
Status: **Resolved**  
Fix commit: `c77172e`  
Related: [CRITICAL_SIGNATURE_PERSISTENCE_INVESTIGATION.md](CRITICAL_SIGNATURE_PERSISTENCE_INVESTIGATION.md) (Perimeter check P1)  
Severity: **Medium** (default CLI path) · **High candidate** if any client posts Identity via legacy `/events`

**Decision:** Option **A** — reject identity fields on legacy `POST /events`; Identity commits only via `POST /v1/events`.

---

## Resolution

| Item | Detail |
|---|---|
| Boundary | `src/api/events.rs` — `reject_identity_fields_on_legacy` before `submit_event` |
| Error | `LedgerError::IdentityNotSupportedOnLegacyPath` → HTTP **400** + existing `{ "error": "…" }` shape |
| Fields rejected | `identity_key_id`, `identity_signature`, `identity_fingerprint` (any present → reject; not silently dropped) |
| Unchanged | v1 Identity flow, Identity protocol, DB schema, CLI (CLI sends no identity fields) |
| Tests | `tests/legacy_events_identity_reject.rs` + unit check in `src/api/events.rs` |

---

## 1. Where legacy `/events` accepted identity fields (pre-fix)

| Layer | Location | Behavior |
|---|---|---|
| HTTP route | `src/main.rs` → `.nest("/events", api::events::router(…))` | Legacy write surface used by CLI |
| Handler | `src/api/events.rs` → `Json(req): Json<SubmitEventRequest>` | Deserialized body with **no** Identity-specific validation |
| Request type | `src/models/event.rs` → `SubmitEventRequest` | Optional Identity columns are part of the serde struct |
| Persist | `src/service/ledger.rs` → `insert_event_in_tx` | Bound those columns straight into `INSERT INTO events (… identity_key_id, identity_signature, identity_fingerprint)` |

Call chain (before):

```
POST /events
  → events::handler (no Identity checks)
  → ledger::submit_event
  → insert_event_in_tx  // binds identity_* as provided
```

Call chain (after Option A):

```
POST /events
  → events::handler
  → reject_identity_fields_on_legacy  // 400 if any identity_* present
  → ledger::submit_event
  → insert_event_in_tx
```

Contrast — Identity-aware path (unchanged):

```
POST /v1/events
  → validate_submit_request
  → require_feature(Feature::Identity)   // if identity_signature present
  → IdentitySigningService::validate_and_prepare  // PoP / key ownership / not revoked
  → build SubmitEventRequest with *verified* fields only
  → insert_event_in_tx
```

---

## 2. Fields a caller could send directly on `POST /events` (pre-fix)

From `SubmitEventRequest` (all optional except the usual ledger fields):

| JSON field | Type | What happened on legacy |
|---|---|---|
| `identity_key_id` | `Uuid?` | Stored on the event row if present |
| `identity_signature` | `String?` | Stored as opaque text — **not** checked as Ed25519 over the leaf hash |
| `identity_fingerprint` | `String?` | Stored as opaque text — **not** recomputed from a registered key |

Also present on the same struct (not Identity, but relevant to dual-path confusion):

| Field | Legacy behavior |
|---|---|
| `event_id` | Optional; used if set |
| `parent_event_id` | Accepted in JSON but **ignored** at insert (parent = current chain head) |

There was **no** requirement that:

- the account has `identity_enabled`;
- the key belongs to the account / is verified / is not revoked;
- the signature matches `MerkleTree::build_leaf(sequence, event_id, parent, file_hash)`.

Downstream read/verify paths that trust `events.identity_*` could then surface misleading Identity claims for events created this way.

---

## 3. Why CLI is unaffected

`evident commit` → `EvidentClient::submit_event` (`src/client.rs`) posts **only**:

```json
{
  "chain_id": "…",
  "parent_event_id": "…",
  "file_hash": "…",
  "idempotency_key": "…"
}
```

It does **not** send `identity_key_id`, `identity_signature`, or `identity_fingerprint`.  
There is still **no** `evident identity` register/sign subcommand for event submit (register/revoke remain Dashboard / HTTP).

So the **default pilot CLI path** does not exercise this finding. Residual exposure was **direct HTTP** (scripts, custom clients, mistaken use of `/events` instead of `/v1/events` for Identity) — now closed by reject-on-legacy.

---

## 4. Applied fix (Option A)

**Policy:** legacy `POST /events` must not accept Identity-bearing submits without the v1 validation stack.

1. If any of `identity_key_id`, `identity_signature`, `identity_fingerprint` is `Some(…)`:
   - return **`400`** with message that Identity event signatures must use `POST /v1/events`.
2. Do **not** implement a second PoP stack on legacy.
3. Do **not** change `SubmitEventRequest` field set used by v1 (v1 still builds verified fields into the same struct for `insert_event_in_tx`).
4. Regression tests: `POST /events` with identity fields rejected; without identity unchanged; `POST /v1/events` with valid PoP still works.

### Alternatives (not chosen)

| Option | Pros | Cons |
|---|---|---|
| A. Reject identity on legacy (**chosen**) | Smallest diff; forces single Identity entry point | Custom `/events` Identity clients must migrate to v1 |
| B. Run same `require_feature` + `validate_and_prepare` on legacy | Keeps dual HTTP surfaces | Duplicates v1 logic; higher regression risk; larger change |
| C. Document-only | Zero code | Leaves the hole open for any HTTP client |

### Out of scope (honored)

- Billing / Paddle / subscription middleware  
- Schema / migrations  
- CLI Identity subcommands  
- Changing v1 Identity protocol  
