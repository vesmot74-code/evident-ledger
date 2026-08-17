# Storage Transparency Audit

**Date:** 2026-08-17  
**Stage:** UX — Storage Transparency  
**Related:** Backup Tiering  
**Base:** post–Backup Tiering + Patch 1 (ownership order / export filename ms)

## Purpose

After Backup Tiering, Free/Legal users can export a cryptographic chain snapshot, but UI copy still framed the action as a “backup” of files. That implied Evident Ledger copies original user files.

This Stage corrects the mental model with terminology and an informational storage block only. No backend, API, snapshot format, or CLI command rename.

## Product message

| Layer | What it is | Where |
| --- | --- | --- |
| Original files | User’s files | Wherever the user put them (never copied by Evident) |
| Application data | Local Evident state | `~/.evident/` (resolved via `evident_dir_path()`) |
| Exported snapshots | Cryptographic evidence JSON | `~/.evident/backups/` (via `backups_dir`) |
| Server backup | Vault+ server-side copy of the snapshot | Evident Ledger infrastructure |

**Rule:** evidence snapshot ≠ copy of original files. Legal weight still comes from TSA and Identity, not from export or backup existence.

## Terminology (Free / Legal)

| Surface | Before | After (RU / EN) |
| --- | --- | --- |
| Screen heading | Локальная резервная копия / Local backup | Экспорт доказательства / Export Evidence Snapshot |
| Primary action | Скачать резервную копию / Download backup | Экспортировать снимок доказательства / Export evidence snapshot |
| In progress | Скачивание... / Downloading... | Экспорт... / Exporting... |
| Top-bar entry | Скачать резервную копию / Download backup | Экспорт доказательства / Export Evidence Snapshot |

Vault / Identity headings and Server Backup actions are unchanged.

## GUI changes (`evident-gui-app`)

1. **Snapshot disclaimer** (all plans, under heading): file does not contain originals; contains hashes, signatures, event order.
2. **Backup Tiering disclaimer** (kept, shown next): restores local data if lost; legal weight from TSA/Identity.
3. **`CollapsingHeader` “Where is my data stored?”** with real paths from `evident_dir_path()` / `backups_dir()`. Vault+ adds the Server backup bullet when `server_backup == true`.

## CLI (`src/bin/evident.rs`)

Command remains `evident backup export`. Help text only: clarifies cryptographic evidence snapshot, not a copy of originals, no server-side storage.

## Out of scope

- Backend / API / DB / JSON schema  
- `Feature::ServerBackup` / create/list/download/restore semantics  
- Restore-from-file  
- App state / WorkerResponse / client methods  
- Cargo dependencies  

## Acceptance

- Free/Legal: export framed as evidence snapshot; paths visible; no “Local backup” wording on Backup screen.  
- Vault+: Server backup heading retained; storage block includes server bullet.  
- RU ↔ EN covers headings, buttons, disclaimers, storage block.  
- Backup Tiering tests remain green; this Stage is UX-only.
