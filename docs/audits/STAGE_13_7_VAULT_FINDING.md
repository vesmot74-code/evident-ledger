# Stage 13.7 Vault Status Finding

Date: 2026-08-02

## Finding

Vault documentation previously described "encrypted server backup".

Implementation review found:

Implemented:

- server-side backup snapshots (`src/service/backup.rs`)
- plan-gated backup functionality (`Feature::ServerBackup`)
- JSON persistence to `EVIDENT_BACKUP_DIR` via `tokio::fs`

Missing:

- encryption-at-rest (no encrypt/AES/cipher/sodium call sites found in
  backup code path or deployment docs)

## Subscriber verification

No production database exists for this project — it is in RC / pre-pilot
stage (per `STAGE_13_7_RELEASE_CANDIDATE.md`), and `docs/DEPLOYMENT.md` contains
only a placeholder `DATABASE_URL` template, no real production connection
string.

Local dev database check (informational only, NOT production validation):

Command:

```text
psql "postgres://ledger:ledger@localhost:5433/ledger" -c
"SELECT COUNT(*) FROM accounts a JOIN subscriptions s ON
s.account_id = a.account_id JOIN tariff_plans tp ON tp.plan_id = s.plan_id
WHERE tp.name = 'vault' AND s.status = 'active';"
```

Result:

```text
ERROR: relation "subscriptions" does not exist
```

Note: the dev schema does not have a table named `subscriptions` under this
name/join path — either the actual schema uses a different table/column
naming, or this dev database has not been migrated with that model. This
error does NOT confirm zero subscribers; it only confirms this specific
query was not runnable against this dev database as written. No further
attempt was made to identify the correct dev schema, since dev-database
subscriber counts would not be production-representative in any case.

Conclusion:

Given the project's pre-pilot / RC-preparation stage and the absence of any
production database, there is no evidence of existing paying Vault
subscribers. This should be explicitly reconfirmed by the project owner
before any customer-facing communication, and before this finding is treated
as fully closed.

Current Vault implementation should not be marketed as encrypted storage
until encryption-at-rest is implemented and verified.
