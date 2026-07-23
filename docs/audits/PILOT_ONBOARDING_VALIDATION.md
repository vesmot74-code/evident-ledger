# Pilot onboarding validation

Date: 2026-07-23  
Commit: `2835cb3` — Stage 13.2 validation after `dff4b6c` (landing CLI CTA) + `a715d67` (dashboard onboarding)  
Environment: production-like (`ENVIRONMENT=production`, `DEV_MODE=false`), `SIGNING_KEY_PATH` pilot key, live `http://127.0.0.1:3000`  
CLI under test: workspace `target/release/evident` (`evident 0.1.0`)  
Release assets reference: GitHub `v1.1.5` (`evident-aarch64-apple-darwin`, not `evident-gui-*`)

## Flow

| Step | Result |
|---|---|
| Landing | **PASS** |
| Registration | **PASS** |
| Dashboard | **PASS** (after onboarding copy fix in this stage) |
| CLI download | **PASS** |
| CLI install docs | **PASS** |
| First commit | **PASS** |
| Verify (CLI) | **PASS** |
| Verify (API `/v1/proof`) | **PASS** |
| Public registry | **PASS** |

Overall: **READY FOR CONTROLLED PILOT** — no Critical product blockers on the CLI proof path. Remaining items are documented Low / ops notes (browser `Secure` cookie on plain HTTP).

---

## Step detail

### Landing

| | |
|---|---|
| Result | **PASS** |
| URL | `http://127.0.0.1:3000/` |
| Expected | Primary CTA = Download CLI → `evident-*`; GUI separate; Login/Register; Pricing (`/#tiers`); Docs/whitepaper |
| Actual | Primary CTA labeled **Download CLI**; default/JS targets `evident-windows-x64.exe` / `evident-aarch64-apple-darwin` / …; **GUI Preview** secondary row only; Login/Sign up present; `#tiers` pricing section present; `/whitepaper` + PDF OK |

Evidence (live HTML after restart of rebuilt binary): primary `el-download-main` href contains `/download/evident-` and not `evident-gui`.

### Registration

| | |
|---|---|
| Result | **PASS** |
| URL | `GET /register` → `POST /auth/register` |
| Expected | Account created; UI usable; session obtainable after login |
| Actual | `curl -I /register` → 200; `POST /auth/register` → **201**; browser form posts JSON then redirects to `/login` (not directly Dashboard) |

Session cookie issued on login: `evident_session=…; HttpOnly; SameSite=Lax; Secure` (see Finding O3).

### Dashboard first visit

| | |
|---|---|
| Result | **PASS** (after Stage 13.2 UX fix) |
| URL | `GET /dashboard/ui` |
| Expected | Empty-state guidance: no proofs yet, install CLI, concrete `evident commit` example; disappears after first commit |
| Actual | Onboarding shows **No proofs yet**, `chmod` / `xattr` / `./evident --version`, API key steps, `evident new-chain` / `commit` / `verify`; plan chip **Free plan** / **No subscription**. After first commit, `data-onboarding="first-run"` absent |

### CLI download / install

| | |
|---|---|
| Result | **PASS** |
| Expected | Asset `evident-aarch64-apple-darwin` (CLI), not GUI; docs match; Gatekeeper / `xattr` documented |
| Actual | `docs/CLI_INSTALLATION.md` lists real CLI assets; Gatekeeper + notarization deferred called out; `./evident --version` works from release build |

### First proof

| | |
|---|---|
| Result | **PASS** |
| Command | `evident new-chain` then `evident commit /tmp/pilot132.txt --chain <id>` |
| Actual | `anchored event=8d8e3f69-9f72-452d-83dc-0dec7df5b0db`; proof JSON written under `~/.evident/proofs/…` |

DB:

```text
event_id=8d8e3f69-9f72-452d-83dc-0dec7df5b0db
length(signature)=128
```

### Verify

| | |
|---|---|
| Result | **PASS** |
| CLI | `evident verify <proof.json>` → `OK: proof valid` |
| API | `GET /v1/proof/{event_id}` → `"proof_status":"anchored"` + signature + TSA |
| Registry | `public_proof_id=pv_Jc5Tts4ZmzRTHKmugjyCyj`, `enabled=t`; materialization row present |

---

## Findings

### O1 — Dashboard empty state lacked concrete CLI commands

| | |
|---|---|
| Severity | **Medium** (first-use friction) |
| Status | **Fixed in Stage 13.2** |
| Description | Stage 13.1 onboarding listed steps but did not show `No proofs yet` or install/`evident commit` commands |
| Recommendation | Implemented: concrete install + commit/verify snippets in dashboard first-run block |

### O2 — Stale server binary served old GUI landing during audit

| | |
|---|---|
| Severity | **High** if operators forget to restart after deploy |
| Status | **Closed for this host** (restarted rebuilt `target/release/evident-ledger`); residual ops note |
| Description | Live process still served `evident-gui-*` until kill + rebuild into workspace `target/` (sandbox `CARGO_TARGET_DIR` had produced a different binary earlier) |
| Recommendation | Pilot checklist: always restart from the intended `SIGNING_KEY_PATH` + freshly built/released binary; confirm landing HTML contains `Download CLI` |

### O3 — `Secure` session cookie with `DEV_MODE=false` on HTTP localhost

| | |
|---|---|
| Severity | **High for browser-only local HTTP**; **Low for HTTPS / CLI-centric pilot** |
| Status | **Open — accepted for controlled pilot with workaround** |
| Description | Login sets `Secure` when `!dev_mode`. Browsers will not store the cookie on `http://127.0.0.1`. Curl/`credentials` scripts still work |
| Recommendation | Real pilot: terminate TLS (or tunnel). Local browser smoke: temporary `DEV_MODE=true` **or** HTTPS. Do not redesign auth in this stage |

### O4 — Register redirects to `/login` instead of Dashboard

| | |
|---|---|
| Severity | **Low** |
| Status | Open (accepted) |
| Description | Extra sign-in step after create account |
| Recommendation | Optional later UX polish; not a blocker |

### O5 — Landing architecture copy still says “evident (CLI / GUI)”

| | |
|---|---|
| Severity | **Low** |
| Status | Open (accepted) |
| Description | Mild naming mix after primary CTA correctly says Download CLI |
| Recommendation | Copy tweak in a later marketing pass; primary CTA already correct |

### O6 — OS detection may mis-label Apple Silicon as Intel in some environments

| | |
|---|---|
| Severity | **Low** |
| Status | Open (accepted) |
| Description | Embedded browser showed “Download CLI for macOS (Intel)” while host is Apple Silicon; alt links still list Apple Silicon CLI asset |
| Recommendation | User can pick **CLI · macOS (Apple Silicon)** alt link; Stage 14 portal can improve detection |

### O7 — Ops: `.env` `DATABASE_URL` can overwrite pilot DB URL

| | |
|---|---|
| Severity | **Medium (ops)** |
| Status | Documented |
| Description | Sourcing `.env` after pilot runtime env pointed the process at DB `ledger` instead of `evident_ledger_pilot_11_6` during this audit |
| Recommendation | Export `DATABASE_URL` last in start scripts; pin pilot DB in a dedicated env file |

---

## Verdict

```
READY FOR CONTROLLED PILOT

Incident / UX remediation verified:
- signature persistence gap resolved (prior)
- legacy identity boundary enforced (prior)
- landing Download CTA → CLI artifact (dff4b6c + live restart)
- dashboard first-run shows concrete CLI commands (this stage)
- first commit → anchored proof + public registry
- CLI verify OK

Residual:
- browser sessions on plain HTTP + Secure cookies (O3) — use HTTPS or DEV_MODE for local browser smoke
```

Next step: **real Pilot onboarding of the first external user** (prefer HTTPS endpoint; free plan; CLI path per [CLI_INSTALLATION.md](../CLI_INSTALLATION.md)).
