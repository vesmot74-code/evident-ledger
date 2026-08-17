# Backup Tiering Audit

**Date:** 2026-08-17
**Stage:** Backup Tiering — Local Export (all plans) + Server Backup (Vault+)
**Base:** post–Public Certificate v2 on `main`

## Purpose

Split the previous single “Backup” path into two product mechanisms:

| Mechanism | Plans | Server persistence |
| --- | --- | --- |
| **Local Export** | Free, Legal, Vault, Identity | **None** — JSON returned to the client only |
| **Server Backup** | Vault, Identity | File under `data/backups/` + row in `backups` |

**Product rule:** a backup restores data if the local file is lost. Legal / evidentiary weight comes from TSA and Identity, not from the existence of a backup.

**Out of scope this Stage:** restore-from-local-file UX (`evident backup restore <file>`), Advanced Evidence Package, TSA/Identity/crypto/PDF changes.

## Previous behavior

- `create_backup()` always required `Feature::ServerBackup`.
- Free/Legal GUI showed “Upgrade required” with no way to obtain a chain snapshot.
- CLI had only `create` / `list` / `download` / `restore` (server-backed).

## New behavior

```text
                    BACKUP
                      │
          ┌───────────┴───────────┐
          │                       │
    LOCAL EXPORT              SERVER BACKUP
      All plans                  Vault+
          │                       │
          ▼                       ▼
   JSON on user machine     JSON on server
          │                       │
          ✕                       ▼
   no backups table         backups table + file
```

## Tariff matrix

| Tariff | TSA | Local Export | Server Backup | Identity |
| --- | --- | --- | --- | --- |
| Free / Base | Machine / Free TSA | ✅ | ❌ | ❌ |
| Legal TSA | Qualified (jurisdiction) | ✅ | ❌ | ❌ |
| Vault | as Legal | ✅ | ✅ | ❌ |
| Identity | as Vault | ✅ | ✅ | ✅ |

## Changed files

| File | Change |
| --- | --- |
| `src/service/backup.rs` | `fetch_chain_events()`, `export_local_snapshot()`; `create_backup()` uses shared fetch; uses public `backup_snapshot::BackupSnapshot` |
| `src/service/backup_snapshot.rs` | `sqlx::FromRow` on `EventSnapshot` (unify query mapping) |
| `src/api/backup.rs` | `POST /backup/export` (no server-backup entitlement) |
| `src/client.rs` | `backup_export()` |
| `src/bin/evident.rs` | `evident backup export --chain … [--output …]` |
| `evident-gui-app/src/main.rs` | Free/Legal Local Export UI; Vault+ labeled Server backup; top-bar labels |
| `tests/subscription_enforcement.rs` | Free `POST /backup/export` → 200, no server persist |
| `docs/audits/backup-tiering.md` | this document |

## API

```http
POST /backup/export
Authorization: API key
{ "chain_id": "<uuid>" }

→ 200 application/json
Content-Disposition: attachment; filename="<chain_id>-export-<timestamp>Z.json"
{ "chain_id", "events", "exported_at" }
```

Existing (unchanged semantics):

```http
POST /backup/create          # Vault+ only
GET  /backup/list
GET  /backup/:backup_id
GET  /backup/:backup_id/download
```

## CLI

```bash
evident backup export --chain <uuid> [--output <path>]
# default filename: <chain_id>-export-<timestamp>Z.json
# prints: snapshot exported to <path>
```

Server commands remain Vault+: `create` / `list` / `download` / `restore`.

## GUI

| `server_backup` | Top bar | Screen |
| --- | --- | --- |
| `false` | Download backup | Local backup + **Download backup** |
| `true` | Server backup | Server backup + Create / List / Download / Restore |

Shared disclaimer: backup restores data; legal weight comes from TSA and Identity.

## Security model

- **Local Export:** ownership check only; no `ensure_server_backup_allowed()`; no `fs::write` under `EVIDENT_BACKUP_DIR`; no `INSERT INTO backups`.
- **Server Backup:** still gated by `Feature::ServerBackup`; writes file + DB row.

## Snapshot compatibility

Local Export and Server Backup both serialize `crate::service::backup_snapshot::BackupSnapshot`, compatible with:

- `parse_snapshot()`
- `validate_structural_integrity()`
- `restore_snapshot_bytes()` (server restore path unchanged; local-file restore is a follow-up)

## Tests (run during Stage)

Service (`cargo test --lib service::backup`): includes

- Free `export_local_snapshot` → Ok, no FS/DB persist
- foreign chain → NotFound
- `create_backup` still FeatureNotAvailable without ServerBackup
- existing ownership / list tests

API (`cargo test --test subscription_enforcement backup_`):

- Free `POST /backup/create` → 403 `feature_not_available`
- Free `POST /backup/export` → 200 + no server persist
- Vault create / past_due regression (existing)

Full suite: see agent final report (`cargo test` exit status).

**Actual run (2026-08-17):**

- `cargo test --lib service::backup`: **17 passed**
- `cargo test --test subscription_enforcement backup_`: **4 passed** (incl. Free export 200)
- `cargo test -- --skip dev_tariff_switcher_end_to_end`: Backup Tiering suites green; unrelated failures:
  - `dev_tariff_switcher_end_to_end` — needs live server `:3000`
  - `dashboard_identity_page_requires_session` — expects `/login` but app redirects to `/login?next=…` (pre-existing redirect semantics; not Backup Tiering)

## Known limitations / follow-ups

1. **Restore from local JSON file** not implemented (Option A). Free can download export; restoring that file via GUI/CLI file picker is a separate Stage.
2. GUI unit tests for the two UI branches were not added (manual / label-level verification); architecture does not isolate Backup screen easily.
3. Account screen still shows Backup ON/OFF capability chip (informational); top bar / Backup screen carry the Local vs Server wording.

## Confirmation

> Local Export is not Server Backup.

> Backup does not increase evidentiary/legal weight by itself.
