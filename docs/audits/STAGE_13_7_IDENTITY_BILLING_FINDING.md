# Stage 13.7 Identity Billing Boundary Finding

Date: 2026-08-02

Branch: `stage-13.7-release-candidate`

Status: **Deferred design debt** — no runtime change in this task.

---

## Finding

Identity key **create** and ledger/backup **writes** use different billing gates.
`past_due` blocks paid write capabilities via `write_blocked_by_subscription()`,
but the identity create path under `/accounts` only checks plan entitlement
(`Feature::Identity`).

This allows a `past_due` paid account to register new identity keys while
ledger event writes and backup create are already rejected with `402`.

---

## Current behavior (runtime)

### Shared paid-write gate

| Symbol | Location |
|--------|----------|
| `write_blocked_by_subscription()` | `src/service/subscription_enforcement.rs` |
| Middleware | `src/middleware/subscription_enforcement.rs` |

Blocks when `plan_name != "free"` and `subscription_status == "past_due"`.

Applied to:

- `/v1/*` (including identity list/revoke under `/v1/identity/keys`)
- legacy `/events`, `/chains`
- `/backup` create

Verified siblings: `POST /v1/events`, `POST /backup/create` → `402` on `past_due`.

### Identity create path (affected)

| Item | Value |
|------|-------|
| Module | `src/api/identity_keys.rs` |
| Handlers | `challenge_handler()`, `register_handler()` |
| Gate | `check_identity_entitlement()` → `require_feature(Feature::Identity)` only |
| Routes | `POST /accounts/identity/keys/challenge` |
| | `POST /accounts/identity/keys/register` |
| Mount | `src/api/accounts.rs` → `.nest("/identity/keys", …)` |
| Subscription middleware | **Not applied** on `/accounts/*` |

Create flow:

```text
challenge
    ↓
proof-of-possession
    ↓
register identity key
```

No `subscription_status != past_due` check on this path.

### Identity revoke / read (separate path)

| Item | Value |
|------|-------|
| List | `GET /v1/identity/keys` — `src/api/v1/identity_keys.rs` |
| Revoke | `POST /v1/identity/keys/{id}/revoke` — `src/api/v1/identity_key_revoke.rs` |
| Entitlement helper | Does **not** call `check_identity_entitlement()` |
| Subscription middleware | **Yes** — entire `/v1` router layers `subscription_enforcement_middleware` |

Implications for `past_due` on a **paid** plan:

| Operation | Route | Current past_due outcome |
|-----------|-------|--------------------------|
| Create challenge | `POST /accounts/identity/keys/challenge` | **Allowed** (entitlement only) |
| Register key | `POST /accounts/identity/keys/register` | **Allowed** (entitlement only) |
| List keys | `GET /v1/identity/keys` | **Allowed** (read method) |
| Revoke key | `POST /v1/identity/keys/{id}/revoke` | **Blocked** (`402`) via write middleware |
| Verify identity signatures | verify / event paths | **Allowed** (reads / non-create) |

**Important correction vs informal assumption:** revoke is *not* currently
billing-independent. It avoids `check_identity_entitlement()`, but because it
is a `POST` under `/v1`, `write_blocked_by_subscription()` still applies.
This task does **not** change revoke implementation.

### Documented exception (today)

Current docs intentionally allow identity create under `past_due`:

- `SECURITY.md` invariant 18 — `/accounts/*` including identity challenge/register remain available
- `docs/BILLING_MODEL.md` §5 — `/accounts/identity/keys/*` listed as allowed exception

This finding records that those docs match `/accounts` create behavior, and
proposes a **future** policy change — not an immediate doc rewrite in this task.

---

## Risk

A non-paying (`past_due`) account on an identity-capable plan can still:

- create new signer identities;
- expand the trusted key surface;
- add new signing sources;

while ledger writes that grow billable usage are already blocked.

Simultaneously, revoke on `/v1` is blocked by the same write gate — which may
hinder security incident response under `past_due` (opposite of the desired
create/revoke asymmetry).

---

## Intended policy (proposal — not implemented)

Do **not** use a blanket identity block.

| Operation | past_due |
|-----------|----------|
| GET/list identity keys | allow |
| verify identity signatures | allow |
| revoke identity key | allow |
| create challenge | **block** |
| register identity key | **block** |

Rationale: revoke must remain available for security incident response;
create expands trust surface and should follow paid-write discipline.

---

## Future implementation boundary

When implemented (separate change):

1. Add billing-state check **only** for:
   - `challenge_handler`
   - `register_handler`
   in `src/api/identity_keys.rs` (and keep mount under `/accounts` or move
   create under a gated surface — design choice deferred).
2. Do **not** block:
   - revoke
   - read/list
   - verification
3. Adjust `/v1` revoke so `past_due` does **not** apply write-block to revoke
   (path exception or dedicated allow-list) — required to match intended table.
4. Update `SECURITY.md` invariant 18 and `BILLING_MODEL.md` §5 in the same
   change set as the runtime fix.

**Out of scope for this finding:** code changes, migrations, middleware refactors.

---

## Decision

Deferred design debt for Stage 13.7 / post-RC.

Runtime behavior unchanged by this document.
