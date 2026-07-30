# Stage 9.x Status Audit — 2026-07-30

## Summary

Stage 9.3 **уже реализован и закрыт тегом** `stage-9.3-identity-signing` (`7dad378`). Выбран **вариант (a)**: байтовый формат `server_signature` не менялся; identity — независимый второй слой (Ed25519 над сырыми 32 байтами `canonical_event_hash` / leaf hash). Вариант (b) (включение identity-метаданных в server signature + version bump) **не реализован**. После 9.3 последовательно закрыты 9.4–9.7 (теги есть). Текущий HEAD (`cfe1c83`) и dirty tree относятся к landing/TSA, не к identity-подписям.

## Git state

- **Последний identity-тег:** `stage-9.7-identity-revoke-ui` (`bc75c58`).  
  Актуальный `git describe`: `pre-landing-freeze-2026-07-28-1-gcfe1c83` (не Stage 9).
- **Теги Stage 9:**
  - `stage-9.0-identity-contract`
  - `stage-9.1-identity-storage`
  - `stage-9.2-identity-registration` → `1efded0`
  - `stage-9.3-identity-signing` → `7dad378`
  - `stage-9.4-identity-verification` → `f517a6a`
  - `stage-9.5-identity-dashboard` → `584dfab`
  - `stage-9.6-identity-revoke` → `685fb2b`
  - `stage-9.7-identity-revoke-ui` → `bc75c58`
- **Коммиты `1efded0..7dad378` (ровно Stage 9.3):** один коммит — `7dad378 feat: add optional user identity signatures to events` (миграция identity-колонок, `identity_signing.rs`, wiring в `submit_event` / `ledger`, тесты).
- **После Stage 9.2 на HEAD:** ~43 коммита (9.3–9.7 + billing/pilot/desktop/landing и т.д.).
- **Незакоммиченные изменения:** **не Stage 9.3.** Это TSA read-path + landing-аудит артефакты:

```text
 M .gitignore SECURITY.md docs/API.md
 M src/api/v1/proof_state.rs src/tsa/* tests/v1_proof.rs vendor/notary-tsa/*
?? docs/audits/STAGE1_STATUS.md
?? docs/design/ADR_TSA_READ_PATH_VERIFICATION.md
?? migrations/20260725000000_tsa_token_verification_cache.sql
?? src/tsa/read_verify.rs tests/tsa_read_verify.rs
```

`git diff --stat` (tracked): 9 files, +209/−116 — TSA verification cache / proof_state, не identity signing.

## Stage 9.2 regression check

**Статус: не сломано** (проверено тестами + чтением кода).

- Маршруты живы: `src/api/accounts.rs` → `.nest("/identity/keys", …)`; в `identity_keys.rs` — `POST /challenge`, `POST /register`.
- `.expect()` на decode challenge сохранён: `hex::decode(&challenge.challenge).expect("challenge stored by create() is always valid hex")`.
- Error mapping challenge: `401` (нет ключа), `403` entitlement, `404` foreign/not found, `409` already used, `410` expired — в `map_challenge_error` / handlers.
- `tests/identity_registration.rs`: **8/8 passed**.

## Stage 9.3 — server_signature format

- **Реализовано:** да (тег + код + тесты).
- **Вариант: (a)** — identity независимый второй слой; `server_signature` не тронут.

### Что подписывает сервер (факт из кода)

`ServerSigner::sign_root` по-прежнему подписывает строку:

```text
{chain_id}:{merkle_root}:{chain_head}
```

Файл `src/signing.rs` — `sign_root` / `verify_root`. Вызов на commit/read path: `src/api/v1/proof_material.rs` → `signer.sign_root(&chain_id, &merkle_root, &chain_head)` / `verify_root(...)`.

В коммите Stage 9.3 (`7dad378`) **не изменялись** `src/signing.rs`, `src/merkle.rs`, `src/api/v1/proof_material.rs` (`git diff 1efded0..7dad378` по этим путям пуст).

### Что подписывает identity (второй слой)

В `submit_event.rs`:

```rust
let canonical_event_hash = MerkleTree::build_leaf(
    planned.sequence, &planned.event_id, &planned.parent_event_id, &file_hash,
);
IdentitySigningService::validate_and_prepare(..., &canonical_event_hash)
```

`MerkleTree::build_leaf` = SHA-256(`sequence || event_id || parent_event_id || file_hash`) — **без** identity metadata.

В `identity_signing.rs` сообщение для Ed25519 — **сырые 32 байта** hex-decoded leaf hash (`verify(..., &raw_hash, signature)`), не конкатенация с key_id/fingerprint.

Identity-поля пишутся в отдельные колонки `events` (миграция `20260718190000_add_identity_signature_to_events.sql`).

### Version bump для identity?

**Нет.** `PROOF_VERSION = "proof_v1"`, `LEAF_VERSION = "leaf_v1"` (`src/proof_format.rs`) — без bump’а под identity. Тест `missing_proof_version_rejected_as_unsupported_format` относится к отсутствию `proof.version` у legacy proof file, не к identity.

### Совместимость со старыми proof

- **Золотых фикстур pre-Stage-9.3 proof JSON в репозитории не найдено** — воспроизвести «архивный» файл proof «как на диске до 9.3» в рамках аудита **не удалось**.
- Косвенные доказательства совместимости формата **server** signature после 9.3:
  - `sign_root` message format в 9.3 не менялся;
  - тест `legacy_event_without_identity_remains_server_verifiable` (identity_signing) — событие без identity проходит `verify_root`;
  - `verify_without_identity_signature_returns_null` — verify без identity-слоя ок;
  - `verify_existing_chain_and_file_contracts_unchanged`.
- **Отдельно (не Stage 9.3):** коммит `1726e82` убрал legacy fallback `merkle_root:chain_head` (без `chain_id`) в `verify_root`. Это ломает **до-versioned** server signatures, но это security-фикс после 9.3, не изменение из-за identity.

**Итог по совместимости Stage 9.3:** по коду — вариант (a), server format не менялся; fixture-based проверка «старого proof файла» — **не проверена** (нет фикстур).

## Verifier readiness (Stage 9.4 preconditions)

| Поверхность | Identity-слой | Server signature |
|---|---|---|
| `GET /v1/verify/{event_id}` | **Да** — `IdentityVerificationService` поверх того же `build_leaf` hash; optional поле `identity_signature` в ответе; отсутствие → `null` | Тот же `verify_root` / chain path |
| CLI `evident-verify` (`src/bin/verify.rs`) | **Нет** — поля identity не парсятся, веток detection нет; только `verify_root` + структура chain | Только текущий versioned server format |

Stage 9.4 **начат и закрыт тегом** `stage-9.4-identity-verification` (`f517a6a`): API verify + `tests/v1_verify_identity.rs` (7/7 ok). Коммит **не** трогал `src/bin/verify.rs`.

`docs/IDENTITY_MODEL.md` §8 помечает 9.4 как «`/v1/verify + verifier CLI`» — **завышение**: CLI identity не умеет. Для offline CLI identity-verify — отдельный gap (если нужен по контракту).

Verifier API умеет и события **без** identity, и **с** identity одним путём (optional extension). CLI — только старый (server-only) путь.

## Build/test state

Рабочее дерево: HEAD `cfe1c83` + dirty TSA/landing. Код в аудите **не менялся**.

### `cargo build`

```text
Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.82s
```

(много warnings; future-incompat sqlx-postgres)

### `cargo test --lib`

```text
test result: ok. 146 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 30.02s
```

### Stage 9 targeted

| Suite | Result |
|---|---|
| `identity_registration` | **8 passed** |
| `identity_signing` | **8 passed** (вкл. `legacy_event_without_identity_remains_server_verifiable`) |
| `v1_verify_identity` | **7 passed** |

### `cargo test -p evident-ledger --no-fail-fast`

Агрегат по suite summary lines: **~532 passed, 25 failed** (41 suite lines).

Падения **не относятся к Stage 9 identity** (identity suites в полном прогоне зелёные):

| Failure cluster | Причина (факт) |
|---|---|
| `dev_tariff_switcher` (1) | live server без `DEV_MODE` |
| `landing` (3) | временная Apple ID landing (`cfe1c83`) ломает CTA/nav expectations |
| `v1_proof` (10), часть `v1_verify*` | dirty TSA read-path: пример `v1_get_proof_valid_signature_returns_anchored` → **500 вместо 200** |

## Открытые вопросы, требующие решения владельца

1. **Вариант a vs b для Stage 9.3 — уже решён в пользу (a).** Новый выбор не нужен, если не планируется пересмотр архитектуры.
2. Нужен ли **CLI identity verification** (чтобы совпасть с формулировкой IDENTITY_MODEL §8), или достаточно API `/v1/verify`?
3. Dirty **TSA read-path** ломает часть proof/verify integration tests на текущем дереве — коммитить / откатить / изолировать до следующих ТЗ?
4. Нужны ли **golden proof fixtures** pre-/post-identity для явной regression-совместимости offline verifier (сейчас их нет)?

## Рекомендация по следующему шагу

1. **Не начинать Stage 9.3 заново** — он done, вариант **(a)**.
2. Считать identity pipeline 9.2→9.7 **закрытым по тегам**; следующий identity-related work — только явные gaps (CLI identity, docs sync).
3. Перед новым ТЗ по proof/verify: разрулить **незакоммиченный TSA** (он сейчас даёт 500 на proof tests) и отдельно landing test drift.
4. Если цель — «Stage 9 полностью по контракту IDENTITY_MODEL»: уточнить у владельца требование к **CLI**, затем отдельное ТЗ Stage 9.4b/9.x CLI — **не** трогая `server_signature` format.
