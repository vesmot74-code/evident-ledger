# Stage 13.7 — RC Ready Gate

Date: 2026-08-02

Branch:

stage-13.7-release-candidate

Current commit:

`6b8a2006658ab36a01d2a4bfea5922bcdb1f96d9`

Target tag:

`v1.2.0-rc1`

---

## 1. Release decision summary

Stage 13.7 RC readiness review completed.

Decision:

READY FOR RC TAGGING

Scope:

- TSA trust hardening
- RFC3161 verification regression fix validation
- startup guard validation
- warnings classification
- documentation lineage cleanup

No release blockers remain.

This document approves **RC tagging readiness only**. It does not grant production release approval or final production approval.

---

## 2. Evidence matrix

| Area                                  | Evidence                                  | Status          |
| ------------------------------------- | ----------------------------------------- | --------------- |
| TSA RFC3161 digest verification       | STAGE_13_7_RELEASE_CANDIDATE.md           | PASS            |
| TSA startup guards                    | STAGE_13_7_TSA_STARTUP_GUARDS_EVIDENCE.md | PASS            |
| Historical startup validation lineage | STAGE_13_7_STARTUP_VALIDATION.md          | PASS            |
| Warnings audit                        | STAGE_13_7_WARNINGS_AUDIT.md              | PASS            |
| Final review scope                    | STAGE_13_7_FINAL_REVIEW.md                | PASS WITH NOTES |

Notes on final review: `STAGE_13_7_FINAL_REVIEW.md` covers PR #3 (`5bbcebc`) merge-ready scope only and predates expanded TSA evidence / warnings audit (see `STAGE_13_7_RELEASE_CANDIDATE.md`).

---

## 3. TSA evidence lineage

TSA startup evidence lineage:

1. `STAGE_13_7_STARTUP_VALIDATION.md`

Historical evidence:

- PR #3 scope
- commit `5bbcebc`
- manual startup validation

2. `STAGE_13_7_TSA_STARTUP_GUARDS_EVIDENCE.md`

Expanded RC evidence:

- complete startup guard matrix
- runtime exit code validation
- expanded scenario coverage

The historical document remains preserved.

The expanded document is the RC evidence reference.

---

## 4. Warnings status

Reference:

`docs/audits/STAGE_13_7_WARNINGS_AUDIT.md`

Warnings audit result:

Total classified:

201

Safe cleanup candidates:

49

Intentional/backlog:

152

Hard blockers:

0

Autofix:

NOT RUN

Rust files changed:

0

---

## 5. Open items review

### TSA

PASS

No remaining TSA blockers.

### Documentation lineage

PASS

Duplicate TSA evidence resolved through explicit document roles.

### Warnings

PASS

No hard blockers.

### Dependency future compatibility

sqlx-postgres v0.7.4 future incompatibility remains tracked.

Classification:

soft future compatibility issue.

Action:

sqlx 0.8+ upgrade backlog.

Source:

`STAGE_13_7_WARNINGS_AUDIT.md`

Note:

This is a reference from the warnings audit document.

`RC_READY.md` does not independently re-run cargo future-incompatibility checks.

---

## 6. RC tagging checklist

Live verification (2026-08-02) against baseline `6b8a200`:

```text
git status
→ clean; branch up to date with origin/stage-13.7-release-candidate

git log origin/stage-13.7-release-candidate -1
→ 6b8a200 docs: clarify Stage 13.7 TSA evidence lineage

git log --oneline | grep abeae66
→ present (TSA evidence)

git log --oneline | grep fa95d7e
→ present (warnings audit)

git log --oneline | grep 6b8a200
→ present (evidence lineage)
```

Before creating `v1.2.0-rc1`:

- [x] Working tree clean
- [x] RC branch pushed
- [x] TSA evidence committed
- [x] Warnings audit committed
- [x] Evidence lineage committed
- [ ] Create RC tag
- [ ] Push tag

---

## Related evidence documents

- `docs/audits/STAGE_13_7_RELEASE_CANDIDATE.md`
- `docs/audits/STAGE_13_7_FINAL_REVIEW.md`
- `docs/audits/STAGE_13_7_TSA_STARTUP_GUARDS_EVIDENCE.md`
- `docs/audits/STAGE_13_7_STARTUP_VALIDATION.md`
- `docs/audits/STAGE_13_7_WARNINGS_AUDIT.md`
