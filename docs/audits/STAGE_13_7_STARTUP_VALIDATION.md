# Stage 13.7 TSA Startup Validation

Date: 2026-07-31

## Production fail-closed validation

Environment:
- APP_ENV=production
- FREETSA_CA_CERT_PATH unset
- FREETSA_UNTRUSTED_CERT_PATH unset

Observed:

TSA trust configuration invalid: missing CA certificate path

Exit code:

1

Result:

PASS — production startup rejects missing TSA trust material.

---

## Non-production warn-and-continue validation

Environment:
- ENVIRONMENT=development
- APP_ENV=development
- FREETSA_CA_CERT_PATH unset
- FREETSA_UNTRUSTED_CERT_PATH unset

Observed:

WARN evident_ledger::tsa::inner::trust_config:
TSA trust configuration incomplete: missing CA certificate path

Evident Ledger running on http://0.0.0.0:3000

Result:

PASS — non-production startup continues with warning.

---

## TSA write-path operational validation

Environment:

- Exact TSA trust environment not captured in this validation record.
- This section records observed RFC3161 stamping activity only.

Observed:

TSA: stamped chain

RFC3161 timestamp flow completed successfully.

Scope:

- Confirms TSA write-path stamping request completed.
- Does not claim TSA read-path cryptographic verification.
- Does not replace CA trust-material validation.

Result:

PASS — existing TSA stamping flow observed operational.

---

## Conclusion

Stage 13.7 TSA startup validation completed.

Verified with captured environment and reproducible output:

- production fail-closed behavior
- non-production warning behavior

Observed separately:

- RFC3161 TSA stamping write-path completed successfully.
- Exact runtime environment for this stamping observation was not captured in this validation record.
- This observation does not claim read-path cryptographic verification.
