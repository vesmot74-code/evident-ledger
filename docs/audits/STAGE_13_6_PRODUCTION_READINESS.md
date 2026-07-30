# Stage 13.6 — Production Readiness

Date: 2026-07-30  
Status: Production readiness audit closed for release baseline.

Repository state:
- Branch: stage-13.6-production-readiness
- Tag: stage-13.6-production-readiness
- Release baseline commit: 6695770

Note:
"Release baseline commit" refers to the code state being audited
(6695770), not the commit that introduces this audit document itself.

## Summary

Stage 13.6 confirms the pilot codebase is ready to treat commit `6695770` as
the release baseline for production readiness evidence.

Upstream hardening from Stage 13.5 remains in force:

- production refuses to auto-create a missing signing key;
- development warns when `SIGNING_KEY_PATH` is unset;
- startup prints public key and SHA-256 fingerprint of the raw Ed25519 seed.

Landing contract regression (Apple ID static replacement) was corrected in
`6695770` by restoring the Evident `static/index.html` contract.

Signing key identity confirmed unchanged:
public key and SHA-256 fingerprint match the established pilot baseline.

## Verified

### Signing identity (pilot baseline)

| Field | Value |
| --- | --- |
| Public key | `fd97921df83d5e4adfa94f30989e93411f17641770446c91b6adc3f5676b156a` |
| SHA-256 fingerprint | `f21dbaf7fa6e6e3b94ce657163f7cc72160f332693cdac8d2ad76602b7be622e` |
| Key path (pilot) | `target/pilot116-key.JBOhAH/signing_key.bin` |

Production missing-key protection verified: absent `SIGNING_KEY_PATH` target
panics with `Production signing key missing:` and does not create a new file.

### Landing page contract restoration

Commit `6695770` restored the Evident landing page contract after an accidental static asset replacement.

Verified:

- authentication navigation marker (`<!--AUTH_NAV-->`) restored
- guest navigation exposes `/login` and `/register`
- authenticated navigation exposes `/dashboard/ui`
- primary download CTA points to CLI artifact
- GUI remains available only as explicitly labeled preview

### Test verification

```bash
cargo test --test landing
```

PASS (3/3): guest auth nav, authenticated dashboard nav, primary Download CLI CTA.

```bash
cargo test -- --skip dev_tariff_switcher_end_to_end
```

Note:
`dev_tariff_switcher_end_to_end` requires an already running local HTTP server on
127.0.0.1:3000. It was verified separately with the development server running.
The skip only prevents external HTTP dependency from affecting the default suite.

Landing contract tests pass without a running HTTP server (`landing::index()`
is exercised in-process). Remaining failures observed in
`legacy_events_signature_persist` depend on a live server on `:3000` and its
runtime signing/TSA environment; they are outside the Stage 13.6 landing
restoration and signing-identity evidence scope.

## Out of scope / Carried forward

The following items are intentionally not part of Stage 13.6:

- production deployment automation
- release packaging and version publication
- operator runbook
- backup and restore procedures
- final pilot release checklist

These items are carried forward to Stage 13.7 Release Candidate preparation.

## Result

Stage 13.6 production readiness evidence is complete for baseline `6695770`.

Gate for Stage 13.7: use this audit as the release evidence attachment; do not
rotate the pilot signing key; keep `SIGNING_KEY_PATH` explicit in production.
