# Stage 12.0 Findings — Pilot Operations Package

Date: 2026-07-23

Status: **STOPPED — awaiting operator decision**

Inventory was performed **before** writing `SIGNING_KEY_OPERATIONS.md`, per Stage 12.0 gate. No fictional backup procedure was authored.

---

## Finding C1 — No verified signing-key backup

| Field | Value |
|---|---|
| Severity | **Critical Operational Gap** |
| Area | Trust anchor / `signing_key.bin` |
| Blocks | Stage 12.0 completion as READY FOR CONTROLLED PILOT (until remediated or explicitly accepted with compensating controls) |

### Current state (verified)

| Item | Observation |
|---|---|
| Active pilot key | `/Users/iuriiveselskii/evident-ledger/target/pilot116-key.JBOhAH/signing_key.bin` |
| Permissions | `-rw-------` (0600), 32 bytes, mtime `2026-07-23 10:53:06` |
| SHA-256 | `f21dbaf7fa6e6e3b94ce657163f7cc72160f332693cdac8d2ad76602b7be622e` |
| Public key | `fd97921df83d5e4adfa94f30989e93411f17641770446c91b6adc3f5676b156a` (matches `GET /identity`) |
| Offline / off-host backup | **Not found** |
| Dedicated backup directory | None (no `backups/` for signing keys; `EVIDENT_BACKUP_DIR` is for **chain** backup JSON, not the signing key) |
| Restore-on-clean-machine drill | **Not possible** — no second copy to restore from |
| Alternate local file | Repo-root `./signing_key.bin` exists but is a **different** key (`sha256=4586b00a…`, mtime 2026-07-09) — **not** a backup of the pilot key |

Search covered: repo tree, `/tmp`, shallow home/Documents/Desktop, `~/.evident/` (contains CLI identity/API material and chain backup artifacts only).

### Impact

```
document → SHA-256 → ledger event → signing_key.bin → proof validity
```

- The server Ed25519 key is the trust anchor for proof signatures verified against the pinned public key (`~/.evident/server_identity.pub` / `GET /identity`).
- Loss or overwrite of the only copy means:
  - offline `evident verify` against the pinned pubkey fails for proofs signed by that key;
  - a newly generated replacement key does **not** validate historical proofs;
  - there is no automated recovery path in the product today.
- Key currently lives only under `target/` (build artifact tree) — high risk of accidental deletion (`cargo clean`, disk wipe, host loss).

This is an **architectural consequence**, not a hypothetical risk.

### What is required before first production / pilot proof of record

Operator must choose **one** of:

1. **Remediate (recommended):** create an offline backup of the pilot key **outside** the host/repo (encrypted volume / secrets vault / air-gapped media), then prove restore:
   - restore to a temp path on a clean check;
   - `sha256` match;
   - derived public key match;
   - `evident verify` of an existing Stage 11.6 proof with the restored key’s public key pinned;
   - only then complete `docs/SIGNING_KEY_OPERATIONS.md` with the **verified** procedure.
2. **Accept risk explicitly:** document that pilot proceeds with a single on-disk copy and no verified off-host backup (not recommended; still must not invent a procedure that was not tested).

### Explicitly not done in this stage

- No code changes.
- No silent copy of the key into the repo or git.
- No placeholder “copy the file somewhere safe” runbook pretending a backup exists.

---

## Stage 12.0 document package status

| Document | Status |
|---|---|
| `PILOT_DEPLOYMENT_CHECKLIST.md` | Not written — blocked pending C1 decision |
| `SIGNING_KEY_OPERATIONS.md` | Not written — blocked (would be fictional without backup) |
| `ROLLBACK_PROCEDURE.md` | Not written — paused with package |
| `PILOT_ONBOARDING_RUNBOOK.md` | Not written — paused with package |
| `MANUAL_MONITORING.md` | Not written — paused with package |
| `PILOT_SLA_DRAFT.md` | Not written — paused with package |
| `PILOT_READINESS_SUMMARY.md` | Not written — paused with package |

Other Stage 11.x limitations (Qualified TSA / paid writes, `paddle_price_id` seed, etc.) remain as previously documented; they are **not** this Critical gap.

---

## Decision needed from operator

Reply with one of:

- **A)** Proceed to create a real off-host backup + restore drill (paths/tools you prefer), then resume Stage 12.0 docs.
- **B)** Accept single-copy risk for controlled pilot and authorize documenting that acceptance (still no fake restore steps).
- **C)** Other compensating control (describe).
