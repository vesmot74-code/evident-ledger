# ADR: Signing Key Governance and Lifecycle Controls

Date: 2026-08-02

Status: Accepted (pilot governance baseline)

## Context

The server signing key is the cryptographic identity of the Evident Ledger
deployment.

The current architecture uses an Ed25519 signing key loaded through
`SIGNING_KEY_PATH`.

Replacing the signing key creates a new signing identity and does not restore
the ability to validate historical proofs against the previous public key.

Therefore key handling requires explicit operational governance.

---

## Existing Controls

### Explicit production key source

Production signing identity is loaded exclusively from:

`SIGNING_KEY_PATH`

The production deployment must not rely on working-directory fallback paths.

Evidence:

- `docs/SIGNING_KEY_OPERATIONS.md`
- `docs/audits/STAGE_13_5_IDENTITY_HARDENING.md`

### No automatic production key generation

Production startup fails when the configured signing key is missing.

The system does not silently create a replacement signing identity.

Evidence:

- `docs/audits/SECURITY_AUDIT_STAGE_11_2.md`
- `docs/audits/STAGE_13_5_IDENTITY_HARDENING.md`

### Backup and restore control

The pilot signing key has:

- off-host backup
- SHA-256 integrity verification
- restore drill evidence

Evidence:

- `docs/audits/STAGE_12_0_FINDINGS.md`
- `docs/SIGNING_KEY_OPERATIONS.md`

---

## Decisions

### D1 — Key replacement requires explicit authorization

A signing key replacement is an operational security event.

Any future replacement must include:

- documented reason;
- authorized operator approval;
- previous key backup confirmation;
- public identity transition record;
- verification impact assessment.

### D2 — Rotation architecture is deferred

Automatic or scheduled signing key rotation is not implemented.

Future rotation design requires a separate architecture decision covering:

- historical proof verification continuity;
- verifier trust transition;
- operator workflow;
- possible HSM/KMS integration.

---

## Rotation review triggers

Rotation architecture review should be started when:

- enterprise compliance requires scheduled rotation;
- HSM/KMS support is introduced;
- multiple production operators are introduced;
- customer-managed keys become supported;
- signing key compromise occurs.

---

## Consequences

Current pilot keeps a stable signing identity.

Operational continuity is preserved through:

- explicit key loading;
- backup controls;
- restore verification.

Future rotation requires a dedicated design phase.
