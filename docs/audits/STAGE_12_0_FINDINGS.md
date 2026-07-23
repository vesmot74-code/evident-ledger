# Stage 12.0 Findings — Pilot Operations Package

Date: 2026-07-23

Status: **C1 Closed — Stage 12.0 docs package resumed**

---

## Finding C1 — No verified signing-key backup

| Field | Value |
|---|---|
| Severity | Critical Operational Gap |
| Status | **Closed** (2026-07-23) |
| Closure reason | Off-host backup created; integrity verified; restore drill passed (load via `evident-ledger` + `evident verify` of existing proof). Active key unchanged; no new key generated; `SIGNING_KEY_PATH` unchanged. |

### Closure evidence (summary)

| Check | Result |
|---|---|
| Backup location class | `$HOME/.evident-ledger-ops/signing-key-backups/…` (outside repo / `target/`) |
| sha256 | `f21dbaf7fa6e6e3b94ce657163f7cc72160f332693cdac8d2ad76602b7be622e` (active == backup) |
| Public key | `fd97921df83d5e4adfa94f30989e93411f17641770446c91b6adc3f5676b156a` |
| Restore load | PASS — printed expected Public key; no `WARNING: created new server signing key` |
| Proof verify after restore pin | PASS — `OK: proof valid` |

Procedure: [SIGNING_KEY_OPERATIONS.md](../SIGNING_KEY_OPERATIONS.md).

### Historical note (discovery state)

At inventory time the only copy of the pilot key was on the application host under `target/…/signing_key.bin`, with no verified off-host backup. Repo-root `./signing_key.bin` was a **different** key — documented as an operational hazard in SIGNING_KEY_OPERATIONS.md.

---

## Other Stage 12.0 notes

No additional Critical/High product defects were opened during ops documentation. Accepted pilot limitations remain listed in [PILOT_READINESS_SUMMARY.md](../PILOT_READINESS_SUMMARY.md).
