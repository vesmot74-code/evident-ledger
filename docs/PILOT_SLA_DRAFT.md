# Pilot SLA / Limits Statement — DRAFT

> **DRAFT — not a legal contract.**  
> Requires human review before showing to a real customer.  
> Describes **current factual** system limits, not new commercial promises.

Related: [BILLING_MODEL.md](BILLING_MODEL.md), [audits/STAGE_11_6_PILOT_SMOKE_TEST.md](audits/STAGE_11_6_PILOT_SMOKE_TEST.md).

---

## Rate limits (public / abuse controls)

In-memory, per-instance, IP-based (mitigation — not a DDoS guarantee). Observed on `/public/verify` in Stage 11.6 (`x-ratelimit-*` headers).

| Scope | Limit (code defaults) | Window |
|---|---|---|
| Public verify | 100 requests | 60 seconds |
| Public certificate | 20 requests | 60 seconds |
| Registration | 10 requests | 60 seconds |
| Login | 10 requests | (login limiter config) |

Tariff `rps_limit` values also exist on plans (see below) for account-level throttling semantics as implemented in the product.

---

## Monthly commit / TSA limits by plan

From seeded `tariff_plans` (migration defaults; confirm live DB):

| Plan | Monthly commits | Monthly TSA | Notes |
|---|---|---|---|
| free | 100 | 100 | Machine TSA |
| legal | 5 000 | 5 000 | Qualified TSA **flagged** in catalog |
| vault | 50 000 | 50 000 | + server backup |
| identity | unlimited (`NULL`) | unlimited (`NULL`) | Identity enabled |

**Operational reality (pilot):** paid plans advertise `tsa_mode=qualified`, but the runtime currently treats Qualified TSA as **unavailable** (`tsa_available` only for machine). Event writes on paid plans may return **`500`**. Free-plan commits remain the reliable evidence path for early pilot.

---

## Known product / ops limitations (must disclose)

1. Identity / paid commits may fail with `500` when Qualified TSA provider is unavailable.
2. Active paid → paid plan change is **not** self-service (support path) — [BILLING_MODEL.md](BILLING_MODEL.md).
3. No CLI command for identity key register/revoke — Dashboard / HTTP only.
4. No dedicated `/health` endpoint (manual checks — [MANUAL_MONITORING.md](MANUAL_MONITORING.md)).
5. Checkout requires ops-seeded `paddle_price_id` values.
6. Offline verify requires correct pinned server public key.
7. Migrations are forward-only (no down migrations) — [ROLLBACK_PROCEDURE.md](ROLLBACK_PROCEDURE.md).

---

## Availability / support (draft placeholders)

| Topic | Draft stance for pilot |
|---|---|
| Support hours | Best-effort / operator-attended during agreed pilot window |
| RPO / RTO | Not contractually defined; DB dumps + signing-key backup are operator-owned |
| Uptime % | Not promised in this draft |

Replace this section only after commercial/legal review.

---

## Document control

| Field | Value |
|---|---|
| Status | DRAFT |
| Updated | 2026-07-23 |
| Audience | Internal pilot ops → optional customer preview after review |
