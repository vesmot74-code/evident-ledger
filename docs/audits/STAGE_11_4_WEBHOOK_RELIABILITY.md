# Stage 11.4 Webhook Reliability Audit

Date: 2026-07-23

Closes **H2** from [SECURITY_AUDIT_STAGE_11_2.md](SECURITY_AUDIT_STAGE_11_2.md).

Paddle remains the only retry mechanism (HTTP non-2xx). No retry queue, worker, or schema changes.

---

## Error Classification

| Error | Class | HTTP | Paddle Retry |
|---|---|---|---|
| Invalid HMAC signature | Permanent | `401 invalid_signature` | No |
| Malformed JSON body | Permanent | `400 invalid_payload` | No |
| Missing required payload field (e.g. `customer_id`) | Permanent | `400 invalid_payload` | No |
| Unsupported / unknown event type | Permanent | `200 ignored` | No |
| Payload hash conflict (same `event_id`, different body) | Permanent | `409 conflict` | No |
| `PlanNotFound` (price↔tariff mapping missing) | **Temporary** | `500 temporary_failure` | Yes |
| Database / connection / transaction failure | Temporary | `500 temporary_failure` | Yes |
| Lost claim race / in-flight `processing` | Temporary | `500 temporary_failure` | Yes |

### Classification rule

- **Permanent** = bad structure / authenticity of the inbound Paddle payload (or intentional ignore).
- **Temporary** = internal system state or infrastructure (including missing tariff price mapping that may sync later).

---

## Implementation Changes

### `src/paddle/processor.rs`

- `WebhookError::is_temporary()` / `error_type_name()`.
- Existing row handling:
  - `processed` / `waiting_for_account_link` → idempotent (unchanged).
  - `failed` / `received` → **reprocess** via existing `mark_processing` (conditional `UPDATE … WHERE status IN ('received','failed')`).
  - `processing` → temporary error (another delivery in flight).
- Concurrent retries: only one transaction wins the conditional UPDATE; loser re-reads status → `Idempotent` if winner finished, else temporary `5xx`.

### `src/api/paddle_webhook.rs`

- Maps temporary errors → `500` + structured log (`event_id`, `event_type`, `error_type`).
- Maps permanent processing errors → `4xx` (signature/malformed contracts unchanged).
- Does not log secrets / payment payloads.

### Not changed

- `paddle_webhook_events` schema / columns
- Signature verification
- Event type mapping / billing apply logic
- Subscription enforcement (Stage 11.3)
- Dashboard / CLI / Identity

---

## Tests

In `tests/paddle_webhook.rs`:

| Test | Covers |
|---|---|
| `invalid_signature_rejected_without_db_changes` | Permanent signature reject |
| `malformed_payload_is_permanent_bad_request` | Permanent malformed JSON |
| `missing_customer_id_is_permanent_bad_request` | Permanent missing field |
| `unrecognized_event_type_is_ignored_with_200` | Permanent ignore |
| `plan_not_found_is_temporary_internal_error` | Temporary → 5xx + `failed` row |
| `failed_webhook_retries_successfully_after_config_fix` | `failed` → reprocess → `processed` |
| `concurrent_failed_retries_do_not_double_apply` | Race-safe claim |
| `duplicate_event_is_idempotent` | Idempotency regression |

---

## Security Impact

- Temporary billing apply failures no longer stuck forever with unhelpful retries that never re-enter processing.
- Permanent bad payloads do not trigger endless Paddle retries.
- No new secret surfaces; logs omit webhook secrets and payment details.
