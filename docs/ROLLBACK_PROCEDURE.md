# Rollback Procedure (Pilot)

Minimal recovery if a deploy misbehaves. Related: [DEPLOYMENT.md](DEPLOYMENT.md), [SIGNING_KEY_OPERATIONS.md](SIGNING_KEY_OPERATIONS.md), [audits/STAGE_11_4_WEBHOOK_RELIABILITY.md](audits/STAGE_11_4_WEBHOOK_RELIABILITY.md).

---

## 1. Stop the service safely

Prefer a clean SIGTERM so in-flight HTTP handlers can finish (Axum/Tokio shutdown on process signal). Avoid `kill -9` unless the process is wedged.

```bash
# Identify listener (port is hardcoded 3000)
lsof -nP -iTCP:3000 -sTCP:LISTEN

# Graceful stop
kill -TERM <pid>
# wait until port is free
lsof -nP -iTCP:3000 -sTCP:LISTEN || echo 'stopped'
```

Do **not** delete or rotate `SIGNING_KEY_PATH` during rollback. Do **not** drop the database.

---

## 2. Return to previous binary / commit

```bash
cd /path/to/evident-ledger
git log --oneline -10          # identify last known-good commit
git checkout <known-good-sha>  # or keep a copied previous release binary aside

cargo build --release --bin evident-ledger --bin evident
# Or restore a previously archived target/release/evident-ledger from the known-good build
```

Keep the **same** `DATABASE_URL`, `SIGNING_KEY_PATH`, and Paddle secrets unless the incident is specifically misconfiguration.

Restart:

```bash
ENVIRONMENT=production DEV_MODE=false \
  # …same env as before…
  ./target/release/evident-ledger
```

Confirm Public key unchanged (`curl -s http://127.0.0.1:3000/identity`).

---

## 3. Migrations — no `down` migrations

**Limitation (current project):** migration files under `migrations/` are **forward-only**. There are **no** `down` SQL scripts and no supported `sqlx migrate revert` workflow in this repository.

| Situation | Action |
|---|---|
| New code fails, **no** new migration was applied | Roll back binary only (step 2). |
| New migration **was** applied, then rollback needed | **Do not invent downs in an incident.** Restore from a **pre-migrate database backup**, or keep the newer schema if it is backward-compatible with the older binary (rare — verify). Prefer taking a Postgres dump **before** `sqlx migrate run` on production-like hosts. |

```bash
# Before applying migrations on a pilot host (recommended)
pg_dump "$DATABASE_URL" -Fc -f "evident_pilot_pre_migrate_$(date -u +%Y%m%dT%H%M%SZ).dump"
```

Creating `down` migrations is **out of scope** for Stage 12.0.

---

## 4. Webhooks during the outage

Paddle retries undelivered / failed deliveries. Stage 11.4 behavior:

- Temporary failures → HTTP `500` + row can be reprocessed (`received` / `failed` → processing).
- Permanent failures → `4xx`; do not expect infinite retry to “fix” bad payloads.
- Idempotency: duplicate event IDs do not double-apply tariff changes.

After rollback + restart:

1. Check recent rows in `paddle_webhook_events` (`status`, `event_type`, timestamps).
2. In Paddle dashboard, confirm notification delivery success / retry state for the destination.
3. Do **not** manually replay signed payloads unless you understand HMAC verification — prefer Paddle “replay” from the dashboard when needed.

Details: [audits/STAGE_11_4_WEBHOOK_RELIABILITY.md](audits/STAGE_11_4_WEBHOOK_RELIABILITY.md).

---

## 5. Post-rollback checks

```bash
curl -s -o /dev/null -w '%{http_code}\n' http://127.0.0.1:3000/
curl -s http://127.0.0.1:3000/identity
# Optional: evident verify of a known proof (pinned server_identity.pub)
```

See [MANUAL_MONITORING.md](MANUAL_MONITORING.md).
