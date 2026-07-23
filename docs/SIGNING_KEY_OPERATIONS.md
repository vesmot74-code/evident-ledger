# Signing Key Operations (Pilot)

**Last verified restore drill:** 2026-07-23 (UTC)  
**Status:** Off-host backup exists; restore drill passed.

Related: [DEPLOYMENT.md](DEPLOYMENT.md), [audits/STAGE_12_0_FINDINGS.md](audits/STAGE_12_0_FINDINGS.md).

---

## Trust model (why this matters)

```
document → SHA-256 → ledger event → signing_key.bin → proof signature
```

Offline `evident verify` checks the proof signature against a **pinned** server public key (`~/.evident/server_identity.pub`, initially from `GET /identity`).

**Loss of the private key without a usable backup means proofs signed by that key can no longer be validated against the expected public identity.** A newly generated replacement key does not resurrect historical proofs. This is a direct consequence of the current architecture — not a hypothetical edge case.

---

## Active pilot key location

| Item | Value |
|---|---|
| Env var | `SIGNING_KEY_PATH` (**exclusive** production source) |
| Stage 11.6 / current pilot path | Absolute path under the deployment host pointing at `…/pilot116-key.*/signing_key.bin` (set via env; do not rely on CWD) |
| Format | 32-byte Ed25519 seed, mode `0600` |
| Public key (hex) | `fd97921df83d5e4adfa94f30989e93411f17641770446c91b6adc3f5676b156a` |
| SHA-256 (last verified) | `f21dbaf7fa6e6e3b94ce657163f7cc72160f332693cdac8d2ad76602b7be622e` |

Confirm live identity:

```bash
curl -s http://127.0.0.1:3000/identity
# {"algorithm":"ed25519","public_key":"fd97921d…156a"}
```

---

## Operational hazard — unmanaged `./signing_key.bin`

A **different** `signing_key.bin` may exist in the repository working directory (dev leftover). It is **not** a backup of the pilot key (different SHA-256).

> **Never place an unmanaged `signing_key.bin` next to the working deployment path.**  
> Production key source is exclusively **`SIGNING_KEY_PATH`**.

If `SIGNING_KEY_PATH` is unset in development, the process falls back to CWD `./signing_key.bin` — that path must not be confused with the pilot trust anchor.

---

## Backup location (off-host)

Backup is stored **outside** the application repository and outside `target/`:

| Property | Guidance |
|---|---|
| Class | Operator-controlled directory under `$HOME/.evident-ledger-ops/signing-key-backups/…` (or equivalent vault / encrypted volume) |
| Permissions | Directory `0700`, key file `0600` |
| Sidecar | `MANIFEST.txt` with sha256 + public key only (no private key duplication beyond the backup file itself) |
| Not in git | Never commit `signing_key.bin` (already gitignored) |

Exact host paths are operator-local; do not publish private-key bytes. Operators should record the backup location in their own secrets inventory.

---

## Mandatory backup before first production proofs

1. Ensure the **active** key is the intended pilot key (`SIGNING_KEY_PATH` + `/identity` public key).
2. Copy **only** that file to the off-host backup location (do not generate a new key).
3. Verify:

```bash
shasum -a 256 "$SIGNING_KEY_PATH"
shasum -a 256 /path/to/off-host/signing_key.bin
# must be identical
```

4. Confirm derived / live public key still matches `fd97921d…156a`.
5. Record date + sha256 in the backup manifest.

---

## Restore procedure (new host or key-path recovery)

1. Stop the service (see [ROLLBACK_PROCEDURE.md](ROLLBACK_PROCEDURE.md)).
2. Create the destination directory; set permissions `0700` / file `0600`.
3. Copy the backup file to the path that will be used as `SIGNING_KEY_PATH`.
4. **Do not** change `SIGNING_KEY_PATH` casually; point it at the restored file if the path must move.
5. Integrity:

```bash
shasum -a 256 "$SIGNING_KEY_PATH"   # must match manifest
```

6. Load check (existing binary mechanism) — expect the known Public key and **no** `WARNING: created new server signing key`:

```bash
ENVIRONMENT=production DEV_MODE=false \
  SIGNING_KEY_PATH=/absolute/path/to/restored/signing_key.bin \
  # plus required Paddle + DATABASE_URL …
  ./target/release/evident-ledger
```

7. Pin CLI trust and verify an existing proof:

```bash
mkdir -p ~/.evident
curl -s http://127.0.0.1:3000/identity | jq -r .public_key > ~/.evident/server_identity.pub
evident verify /path/to/existing/proof.json
# Expect: OK: proof valid
```

8. Re-run a restore drill after any host migration; update **Last verified restore drill** at the top of this document.

---

## What if the key is lost with no backup?

- Proofs signed under the lost key **fail** offline verify against the historical public key.
- Generating a new key creates a **new** identity; it does not validate old proofs.
- Treat as a **Severity Critical** incident: stop issuing new proofs under a replacement key until legal/ops decide how to communicate trust break to verifiers.

---

## Restore drill record (2026-07-23)

| Step | Result |
|---|---|
| Off-host copy created | PASS |
| sha256 active == backup | PASS (`f21dbaf7…622e`) |
| Public key | PASS (`fd97921d…156a`) |
| Load via `evident-ledger` + restored `SIGNING_KEY_PATH` | PASS (no new-key WARNING) |
| `evident verify` of Stage 11.6 proof | PASS (`OK: proof valid`) |
| Active key / live `SIGNING_KEY_PATH` unchanged | PASS |
