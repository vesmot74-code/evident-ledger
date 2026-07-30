# Stage 13.7 — Release Candidate

**Status:** RC: TSA blocker resolved, pending final release checklist  
**Date:** 2026-07-30  
**Branch:** `stage-13.7-release-candidate`  
**Baseline context:** Stage 13.6 audit final `2188194` / production-readiness tag `stage-13.6-production-readiness` @ `6695770`  
**TSA fix:** `5681d3c` — `fix(tsa): align RFC3161 verification digest semantics`

This document is the **RC decision source of truth**.  
Working investigation history for the TSA item lives in:

`docs/audits/STAGE_13_7_TSA_INVESTIGATION.md`

---

## Known Limitations

### Discovery (Stage 13.7 investigation)

- **RFC3161 verification regression (discovered):** valid TSA tokens may fail verification when full CA-backed verification is enabled (`FREETSA_CA_CERT_PATH` / `FREETSA_UNTRUSTED_CERT_PATH`). Root cause (diagnosed): read-path OpenSSL uses `ts -verify -data` on raw merkle-root bytes while write-path stamps a digest imprint; OpenSSL re-hashes and reports message imprint mismatch. Without CA env, OpenSSL is skipped (`unavailable`) and the failure is masked.  
  **Investigation status at discovery:** open (diagnosis complete; fix not yet implemented).  
  **Detail:** `docs/audits/STAGE_13_7_TSA_INVESTIGATION.md`.

### TSA RFC3161 verification regression — resolved

Finding:

Stage 13.7 investigation identified a production logic regression in the TSA
read verification path.

Root cause:

Write-path stored RFC3161 imprint as digest bytes, while OpenSSL verification
used `ts -verify -data`, causing OpenSSL to hash the digest bytes again and
produce an imprint mismatch.

Fix:

Commit `5681d3c` changed the verification path to use digest semantics
(`openssl ts -verify -digest`) matching the RFC3161 write-path.

Verification:

- legacy_events_signature_persist without FREETSA_*: PASS (verification skipped, not a full check)
- legacy_events_signature_persist with FREETSA_*: PASS, verification_status=verified
- tsa_read_verify: PASS
- v1_verify: PASS
- v1_proof: PASS
- v1_events_idempotency: PASS

Status:

Resolved. RC blocker removed.

---

## Environment / exposure note (checked scope only)

No evidence of `FREETSA_*` configuration was found in the local repository’s `.env` / docker config files. Local developer shell may have smoke trust paths set. Deployment/CI environments were not checked. No production exposure is currently expected because this project is still in pre-pilot / RC preparation stage.

---

## RC decision

**RC: TSA blocker resolved, pending final release checklist**

Final release approval is a separate stage. Do not treat this document as RC approved.
