# Evident Ledger — Identity Model

**Status:** Frozen at Stage 9.0 (Identity Contract Freeze).

This document defines user-owned cryptographic identity: key lifecycle, registration, proof extensions, revocation, entitlement, and API boundaries. **No implementation code is specified here** — this is the contract for Stages 9.1–9.5.

**Related documents:**

- [SECURITY.md](../SECURITY.md) — security invariants 30–35
- [SYSTEM_CONTRACT.md](../SYSTEM_CONTRACT.md) — user identity binding (Stage 9)
- [docs/AUTH_MODEL.md](AUTH_MODEL.md) — account and API-key authentication (orthogonal)
- [docs/BILLING_MODEL.md](BILLING_MODEL.md) — `identity_enabled` capability gating

**Existing (unchanged by this contract):**

- **Server identity** — `GET /identity` returns the ledger’s Ed25519 signer public key (`src/signing`, `src/api/identity.rs`). This is **not** user identity.
- **CLI local keys** — `~/.evident/identity.key` / `identity.pub` for client-side generation today; Stage 9 registers the **public** key with the account after proof-of-possession.

---

## 1. Ownership Model

```
PRIVATE KEY
    |
    | never leaves device
    v
Client / Local utility (CLI, local tool)
PUBLIC KEY
    |
    v
Evident Ledger (identity_keys table)
```

### Prohibited

- Server-side generation of user private keys
- Storage or transmission of user private keys to the server
- Server signing on behalf of the user (user signatures are client-produced only)

### Allowed

- Client generates Ed25519 keypairs locally
- Server stores **public keys** only, bound to `account_id` after proof-of-possession
- Server verifies user signatures using stored public keys

---

## 2. Storage — `identity_keys`

User identity keys live in a **dedicated table**, not as columns on `accounts`.

```sql
CREATE TABLE identity_keys (
    id UUID PRIMARY KEY,
    account_id UUID NOT NULL REFERENCES accounts(account_id),
    public_key TEXT NOT NULL,
    fingerprint TEXT NOT NULL UNIQUE,
    label TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    verified_at TIMESTAMPTZ NULL,
    revoked_at TIMESTAMPTZ NULL
);
```

### Invariants

| Rule | Detail |
|------|--------|
| **`fingerprint UNIQUE`** | A key cannot be registered twice globally (prevents duplicate registration attacks) |
| **`verified_at`** | Set after successful proof-of-possession. **User self-registration MUST always set `verified_at`.** NULL is allowed only for admin/migration flows (explicit, out-of-band) |
| **`revoked_at`** | When set, key **cannot** be used for **new** signatures; key **remains** available for verification of **historical** proofs |

**Fingerprint:** derived deterministically from the public key (exact algorithm fixed at implementation time in Stage 9.1; MUST be stable for verification).

---

## 3. Registration Flow — Proof-of-Possession (Mandatory)

Registration of a public key **without** proof-of-possession **MUST** be rejected for user-facing flows.

```
1. Client generates keypair locally
        |
        v
2. Client requests challenge (authenticated)
        |
        v
3. Server returns random challenge bytes (single-use, time-limited)
        |
        v
4. Client signs challenge with private key (local)
        |
        v
5. Client submits public_key + signature + challenge_id
        |
        v
6. Server verifies signature with public_key
        |
        v
7. identity_keys row created, verified_at = now()
```

**Without valid proof-of-possession:** registration returns error; no row with `verified_at` for self-service paths.

---

## 4. API Architecture — CLI First

Identity registration **MUST** be available through the API. Dashboard is a **presentation layer only**. CLI clients remain **first-class** identity clients.

### Primary API (CLI and integrations)

| Method | Path | Auth |
|--------|------|------|
| `POST` | `/accounts/identity/keys/challenge` | `X-API-KEY` |
| `POST` | `/accounts/identity/keys/register` | `X-API-KEY` |

- **Challenge:** server issues nonce; bound to `account_id` from API key
- **Register:** client sends `public_key`, challenge reference, signature; server verifies and persists

**Rule:** Handlers for `/accounts/identity/keys/*` delegate to a shared **service layer**. No duplicate business logic in Dashboard.

### Dashboard (secondary — Stage 9.5)

| Surface | Auth |
|---------|------|
| `/dashboard/identity/keys` (UI) | Session cookie |

Dashboard **MUST NOT** be the only path to register keys. Any Dashboard action **MUST** call the same service functions as the API routes above — never a parallel HTTP call to `/accounts/*` from the browser for identity operations (same pattern as Dashboard API keys in Stage 8.3.1a).

---

## 5. Proof Extension

### Backward compatibility

Proofs **without** `identity_signature` remain **valid**. User signatures are **optional extensions**.

### Extended proof shape

```json
{
  "hash": "...",
  "server_signature": "...",
  "identity_signature": {
    "key_id": "uuid",
    "fingerprint": "...",
    "signature": "..."
  }
}
```

### Why `key_id` (not embedded `public_key`)

- Public key already stored in `identity_keys`
- Smaller proof payload
- Supports key rotation and labeling without rewriting proofs
- Revocation checked via `key_id` → row → `revoked_at` at **signing** time; historical verification uses key as it existed at sign time

### Verifier resolution

```
identity_signature.key_id
        |
        v
identity_keys (lookup public_key, check not deleted)
        |
        v
verify(signature, message, public_key)
```

Revoked keys: **reject new submissions** signed after revocation; **accept verification** of proofs signed before revocation (see §6).

---

## 6. Revocation Policy

When `revoked_at IS NOT NULL`:

- Key **MUST NOT** be used for **new** event signatures or registrations
- Key **MUST** remain resolvable for **verification** of proofs that already contain that `key_id`

**Rationale:** Revoking a key does not annul documents already signed with it — required for audit and legal continuity.

Revocation is **permanent** (no un-revoke in MVP contract).

---

## 7. Entitlement

Uses existing billing capability — **no new billing mechanisms**:

```
tariff_plans.identity_enabled
Feature::Identity (service layer)
```

### Gating rules

`identity_enabled` gates:

- Registration of **new** identity keys (challenge + register)
- Creation of **new** user signatures on submit

`identity_enabled` does **NOT** gate:

- Verification of **existing** proofs (with or without `identity_signature`)
- Resolution of historical `key_id` for offline/API verify

**Capability downgrade:** If an account loses Identity entitlement, old proofs remain verifiable; new identity key registration and new user signatures are blocked until entitlement returns.

---

## 8. Roadmap — Stage 9

```
Stage 9.0 — Identity Contract Freeze          ← this document
        |
        v
Stage 9.1 — Identity Key Storage
        (migration + models + repository)
        |
        v
Stage 9.2 — Challenge Registration
        (POST /accounts/identity/keys/challenge + register)
        |
        v
Stage 9.3 — User Signed Events
        (identity_signature in submit pipeline)
        |
        v
Stage 9.4 — Verification Extension
        (/v1/verify + verifier CLI)
        |
        v
Stage 9.5 — Identity Dashboard
        (/dashboard/identity/keys UI)
```

---

## 9. Security Summary

Normative invariants: [SECURITY.md](../SECURITY.md) §2.5 items 30–35.

| # | Invariant |
|---|-----------|
| 30 | User private keys generated and stored locally; never transmitted |
| 31 | Server never stores user private keys |
| 32 | User signatures optional; proofs valid without them |
| 33 | Revocation permanent; does not invalidate existing proofs |
| 34 | `identity_enabled` gates registration and new user signatures |
| 35 | Existing proofs remain verifiable after capability changes |

---

## 10. Explicit Non-Goals (Stage 9.0)

- Implementation code, migrations, or tests (Stages 9.1+)
- Email-based identity linking
- Server-side key generation or escrow
- Changes to `GET /identity` (server signer)
- New billing SKUs beyond existing `identity` plan / `identity_enabled`
