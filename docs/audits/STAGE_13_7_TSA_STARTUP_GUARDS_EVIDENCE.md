# Stage 13.7 TSA Startup Guards Evidence

Manual RC runtime validation of startup guards and FreeTSA trust configuration.

## Environment

```
Project:
evident-ledger

Branch:
stage-13.7-release-candidate

Commit:
aaa9d3e

OS:
macOS

Rust:
rustc 1.96.0 (ac68faa20 2026-05-25)

Cargo:
cargo 1.96.0 (30a34c682 2026-05-25)
```

Binary under test: `./target/debug/evident-ledger` (built from the commit above).

### Environment variables used during validation

```
FREETSA_CA_CERT_PATH=/tmp/freetsa-trust/cacert.pem
FREETSA_UNTRUSTED_CERT_PATH=/tmp/freetsa-trust/tsa.crt
SIGNING_KEY_PATH=/tmp/freetsa-trust/dummy_signing_key.bin
```

Paddle placeholders (non-secret, evidence-only):

```
PADDLE_API_KEY=rc-evidence-placeholder
PADDLE_WEBHOOK_SECRET=rc-evidence-placeholder
PADDLE_CLIENT_TOKEN=rc-evidence-placeholder
```

`DATABASE_URL` was sourced from the local developer `.env` only for scenarios that proceed past config/TSA (network bind failure; TSA warn-and-continue in development). Its value is not recorded here.

**Placeholder signing key note:**  
`/tmp/freetsa-trust/dummy_signing_key.bin` was used only as a placeholder to satisfy earlier startup guards (config / signing-path presence). **This is NOT a production signing key.**

---

## TSA Trust Chain Verification

Command:

```bash
openssl verify \
  -CAfile /tmp/freetsa-trust/cacert.pem \
  /tmp/freetsa-trust/tsa.crt
```

Observed result:

```
/tmp/freetsa-trust/tsa.crt: OK
```

Trust chain (operational model):

```
FreeTSA Root CA
        |
        v
FreeTSA TSA Signer
        |
        v
RFC3161 Timestamp Token
```

---

## Startup Guard Execution Order

Note:

Startup guards execute sequentially:

1. Configuration validation
   - DEV_MODE rules
   - SIGNING_KEY_PATH requirement
   - Paddle required secrets (non-test builds)
2. TSA trust validation
   - FREETSA_CA_CERT_PATH
   - FREETSA_UNTRUSTED_CERT_PATH
3. Signing key load (production file must exist)
4. Database pool
5. Network binding

Each scenario below was validated independently by satisfying previous guards with required placeholder configuration values.

---

## Manual Validation Matrix

| Scenario                               | Exit code | Result |
| -------------------------------------- | --------: | ------ |
| DEV_MODE enabled in production         |         4 | PASS   |
| Missing SIGNING_KEY_PATH in production |         4 | PASS   |
| Invalid TSA trust configuration        |         1 | PASS   |
| Network bind failure                   |         5 | PASS   |
| TSA config in dev mode                 |      none | PASS   |

---

## Scenario details

### 1) DEV_MODE enabled in production

**Command:**

```bash
env -u SIGNING_KEY_PATH -u FREETSA_CA_CERT_PATH -u FREETSA_UNTRUSTED_CERT_PATH \
  ENVIRONMENT=production APP_ENV=production DEV_MODE=true \
  PADDLE_API_KEY=rc-evidence-placeholder \
  PADDLE_WEBHOOK_SECRET=rc-evidence-placeholder \
  PADDLE_CLIENT_TOKEN=rc-evidence-placeholder \
  ./target/debug/evident-ledger
```

**Env (effective):** `ENVIRONMENT=production`, `APP_ENV=production`, `DEV_MODE=true`, Paddle placeholders; `SIGNING_KEY_PATH` / FreeTSA paths unset.

**stderr:**

```
STARTUP_ERROR config: DEV_MODE cannot be enabled in production environment
```

**stdout:** _(empty)_

**Exit code:** `4`

**Result:** PASS — controlled config exit; no stacktrace.

---

### 2) Missing SIGNING_KEY_PATH in production

**Command:**

```bash
env -u SIGNING_KEY_PATH -u FREETSA_CA_CERT_PATH -u FREETSA_UNTRUSTED_CERT_PATH \
  ENVIRONMENT=production APP_ENV=production DEV_MODE=false \
  PADDLE_API_KEY=rc-evidence-placeholder \
  PADDLE_WEBHOOK_SECRET=rc-evidence-placeholder \
  PADDLE_CLIENT_TOKEN=rc-evidence-placeholder \
  ./target/debug/evident-ledger
```

**Env (effective):** production labels; `DEV_MODE=false`; Paddle placeholders; `SIGNING_KEY_PATH` unset.

**stderr:**

```
STARTUP_ERROR config: SIGNING_KEY_PATH must be set in production environment
```

**stdout:** _(empty)_

**Exit code:** `4`

**Result:** PASS — controlled config exit; no stacktrace.

---

### 3) Invalid TSA trust configuration

**Command:**

```bash
env -u FREETSA_CA_CERT_PATH -u FREETSA_UNTRUSTED_CERT_PATH \
  ENVIRONMENT=production APP_ENV=production DEV_MODE=false \
  SIGNING_KEY_PATH=/tmp/freetsa-trust/dummy_signing_key.bin \
  PADDLE_API_KEY=rc-evidence-placeholder \
  PADDLE_WEBHOOK_SECRET=rc-evidence-placeholder \
  PADDLE_CLIENT_TOKEN=rc-evidence-placeholder \
  ./target/debug/evident-ledger
```

**Env (effective):** production labels; placeholder `SIGNING_KEY_PATH`; FreeTSA paths unset (invalid trust config).

**stderr:**

```
TSA trust configuration invalid: missing CA certificate path
```

**stdout:** _(empty)_

**Exit code:** `1`

**Result:** PASS — production fail-closed TSA trust guard; controlled exit; no stacktrace.

---

### 4) Network bind failure

Port `3000` was held by a temporary listener (`Address already in use`). Prior guards were satisfied with valid FreeTSA paths, placeholder signing key, Paddle placeholders, and local `DATABASE_URL`.

**Command:**

```bash
env ENVIRONMENT=production APP_ENV=production DEV_MODE=false \
  SIGNING_KEY_PATH=/tmp/freetsa-trust/dummy_signing_key.bin \
  FREETSA_CA_CERT_PATH=/tmp/freetsa-trust/cacert.pem \
  FREETSA_UNTRUSTED_CERT_PATH=/tmp/freetsa-trust/tsa.crt \
  PADDLE_API_KEY=rc-evidence-placeholder \
  PADDLE_WEBHOOK_SECRET=rc-evidence-placeholder \
  PADDLE_CLIENT_TOKEN=rc-evidence-placeholder \
  DATABASE_URL=<local developer DATABASE_URL> \
  ./target/debug/evident-ledger
```

**stderr:**

```
STARTUP_ERROR network: failed to bind 0.0.0.0:3000: Address already in use (os error 48)
```

**stdout (abbreviated):**

```
Public key: <hex from placeholder key>
Signing key SHA256: <hex from placeholder key>
Signing key path: /tmp/freetsa-trust/dummy_signing_key.bin
Environment: production
Evident Ledger running on http://0.0.0.0:3000
```

**Exit code:** `5`

**Result:** PASS — controlled network exit after earlier guards succeeded; no stacktrace.

---

### 5) TSA config in development (warn-and-continue)

**Command:**

```bash
env -u FREETSA_CA_CERT_PATH -u FREETSA_UNTRUSTED_CERT_PATH \
  ENVIRONMENT=development APP_ENV=development DEV_MODE=false \
  SIGNING_KEY_PATH=/tmp/freetsa-trust/dummy_signing_key.bin \
  PADDLE_API_KEY=rc-evidence-placeholder \
  PADDLE_WEBHOOK_SECRET=rc-evidence-placeholder \
  PADDLE_CLIENT_TOKEN=rc-evidence-placeholder \
  DATABASE_URL=<local developer DATABASE_URL> \
  RUST_LOG=warn \
  ./target/debug/evident-ledger
```

**Observed log (tracing WARN on stdout):**

```
WARN evident_ledger::tsa::inner::trust_config: TSA trust configuration incomplete: missing CA certificate path
…
Evident Ledger running on http://0.0.0.0:3000
```

**TSA guard exit code:** none (process continued listening; stopped with SIGTERM after bind for evidence capture).

**Result:** PASS — non-production warn-and-continue; no process exit from TSA trust guard.

---

## Acceptance Criteria (Part A)

- [x] TSA trust chain confirmed via OpenSSL (`tsa.crt: OK`).
- [x] All startup guards in the matrix have runtime evidence.
- [x] Failures are controlled typed exits / warnings.
- [x] No user-facing panic stacktraces observed.
- [x] Guard execution order documented.
- [x] Placeholder signing key explicitly marked non-production.
- [x] Evidence document created.

## Conclusion

Stage 13.7 RC TSA startup-guard runtime validation: **PASS**.
