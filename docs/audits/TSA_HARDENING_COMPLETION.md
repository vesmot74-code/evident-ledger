# TSA Hardening Completion Report

Дата: 2026-07-30  
Коммит **не выполнен** агентом — дерево подготовлено к ревью владельца.

## Предварительный аудит diff (до правок)

Незакоммиченное состояние на старте:

| Файл | Относится к TSA? | Обязательно? | Заметки |
|---|---|---|---|
| `src/tsa/read_verify.rs` (new) | да | да | imprint + OpenSSL + cache |
| `src/tsa/lib.rs`, `types.rs` | да | да | exports / `TsaVerificationStatus` |
| `src/api/v1/proof_state.rs` | да | да | wiring read-path verify |
| `migrations/20260725000000_tsa_token_verification_cache.sql` (new) | да | да | cache columns |
| `vendor/notary-tsa/src/openssl_provider.rs`, `lib.rs` | да | да | `verify_tsr_bytes`, `freetsa_trust_paths` |
| `tests/tsa_read_verify.rs`, `tests/v1_proof.rs` | да | да | cache / status assertions |
| `docs/design/ADR_TSA_READ_PATH_VERIFICATION.md` (new) | да | да (документация решения) | |
| `SECURITY.md`, `docs/API.md` | да | да (документация поведения) | `verification_status` |
| `.gitignore` (`.local-backups/`) | **нет** | нет | landing freeze |
| `docs/audits/STAGE1_STATUS.md` | **нет** | нет | отдельный аудит |
| `docs/audits/STAGE9_STATUS.md` | **нет** | нет | отдельный аудит |

Случайных debug/logging/закомментированных кусков в TSA-diff не найдено. Временного кода (кроме `#[cfg(test)]` hooks) нет.

## Выполнено

### Исправления / завершение в рамках этого ТЗ

1. **Аудит** незакоммиченного TSA WIP — классификация in/out of scope (выше).
2. **Миграция на локальных БД:** колонки кэша отсутствовали на `DATABASE_URL` (из‑за этого live `GET /v1/proof` давал **500** при SELECT новых полей). Применено `sqlx migrate run` → `20260725000000` installed на `DATABASE_URL` (на `TEST_DATABASE_URL` уже было).
3. **Качество TSA path:**
   - удалён мёртвый `stub_attestation_for_root` из `read_verify.rs`;
   - unit-тест env-gate в `proof_state.rs` обёрнут в `Mutex` (без гонок с параллельными тестами);
   - удалён неиспользуемый `FREETSA_CA_CERT_URL` в `openssl_provider.rs`.

### Список файлов для атомарного коммита TSA (in-scope)

```
SECURITY.md
docs/API.md
docs/design/ADR_TSA_READ_PATH_VERIFICATION.md
migrations/20260725000000_tsa_token_verification_cache.sql
src/api/v1/proof_state.rs
src/tsa/lib.rs
src/tsa/types.rs
src/tsa/read_verify.rs
tests/tsa_read_verify.rs
tests/v1_proof.rs
vendor/notary-tsa/src/lib.rs
vendor/notary-tsa/src/openssl_provider.rs
```

### Кратко: зачем каждый файл

| Файл | Причина |
|---|---|
| `read_verify.rs` | Свежая crypto-проверка + race-safe cache |
| `types.rs` / `lib.rs` | Статусы API + re-exports |
| `proof_state.rs` | Единая точка read-path для proof/verify |
| migration | `verified_at`, `verification_status`, `token_sha256` |
| `openssl_provider.rs` | OpenSSL verify TSR bytes + FreeTSA trust paths |
| tests | Cache hit/miss, stub gate, malformed DER |
| ADR / SECURITY / API.md | Контракт и ограничения кэша |

### Подтверждение отсутствия побочных изменений в предлагаемом коммите

Identity / proof format / server signature / chain format / CLI / dependencies / Cargo.lock — **не менялись**.

API.md лишь документирует уже добавляемое поле `tsa.verification_status` (часть этого hardening), без новых endpoints.

## Проверки

### RFC3161 verification path (код)

- Структурный imprint: `notary_tsa::parse_and_validate_tsr` → fail closed.
- Подпись/CA: `verify_tsr_bytes` → OpenSSL `ts -verify`; нет silent success.
- Stub `"stub":true`: только при `DEV_MODE` / `APP_ENV=development`, иначе `failed`.
- Нет CA bundle → `unavailable` (не `verified`).
- Повреждённый DER → `failed`.
- Production path: без `unwrap()`/`expect()`/`panic!` (только в `#[cfg(test)]`).
- Cache: reuse только `verified` + matching `token_sha256` → `verified_cached`.

### Профильные тесты

```text
cargo test --lib tsa::
→ 14 passed; 0 failed

cargo test --test tsa_read_verify
→ 4 passed; 0 failed

cargo test --lib api::v1::proof_state
→ 6 passed; 0 failed
```

### Live TSA verify

```text
FREETSA_CA_CERT_PATH=/tmp/freetsa-trust/cacert.pem
FREETSA_UNTRUSTED_CERT_PATH=/tmp/freetsa-trust/tsa.crt
cargo test -p notary-tsa verify_reply_freetsa_smoke -- --ignored
→ test openssl_provider::tests::verify_reply_freetsa_smoke ... ok
```

(сертификаты скачаны во `/tmp`, в репозиторий не добавлялись)

### Миграции

- Есть: `migrations/20260725000000_tsa_token_verification_cache.sql`
- Additive only (`ADD COLUMN IF NOT EXISTS`) — существующие proof/token rows остаются валидны; cache NULL до первой проверки.
- Локально применена на `DATABASE_URL` и `TEST_DATABASE_URL`.

### Обратная совместимость

- Формат leaf / server signature / proof_version не менялся.
- События без TSA-строки: поведение как раньше (`NotProvided`).
- Stub-токены вне development теперь **явно fail** (ужесточение, задокументировано в ADR) — не silent accept.

### Классификация известных падений вне профильных TSA unit/integration

| Симптом | Класс | Действие в этом ТЗ |
|---|---|---|
| Ранее 500 на live `/v1/proof` из‑за отсутствующих колонок на `DATABASE_URL` | вызвано незавершённой миграцией TSA | миграция применена локально |
| Live `v1_proof` без перезапуска сервера с новым бинарём / без `DEV_MODE` для stub | среда / известный ADR constraint | не чинили несвязанные тесты |
| `landing` / `dev_tariff_switcher` failures | вне scope (landing / DEV_MODE server) | не трогали |

## Git state

Коммит **не создавался**.

### `git status` (на момент отчёта)

```text
 M .gitignore
 M SECURITY.md
 M docs/API.md
 M src/api/v1/proof_state.rs
 M src/tsa/lib.rs
 M src/tsa/types.rs
 M tests/v1_proof.rs
 M vendor/notary-tsa/src/lib.rs
 M vendor/notary-tsa/src/openssl_provider.rs
?? docs/audits/STAGE1_STATUS.md
?? docs/audits/STAGE9_STATUS.md
?? docs/design/ADR_TSA_READ_PATH_VERIFICATION.md
?? migrations/20260725000000_tsa_token_verification_cache.sql
?? src/tsa/read_verify.rs
?? tests/tsa_read_verify.rs
?? docs/audits/TSA_HARDENING_COMPLETION.md   # этот отчёт (если сохранён)
```

### `git diff --stat` (tracked)

```text
 .gitignore                                |   3 +
 SECURITY.md                               |   3 +
 docs/API.md                               |  16 ++-
 src/api/v1/proof_state.rs                 | 201 +++++++++++++-----------------
 src/tsa/lib.rs                            |   9 +-
 src/tsa/types.rs                          |  39 ++++++
 tests/v1_proof.rs                         |   8 ++
 vendor/notary-tsa/src/lib.rs              |   3 +
 vendor/notary-tsa/src/openssl_provider.rs |  ~50 +-
```

Плюс untracked in-scope: migration, `read_verify.rs`, ADR, `tests/tsa_read_verify.rs`.

### Предлагаемое сообщение коммита (не выполнено)

```
fix(tsa): harden RFC3161 verification path
```

Рекомендуемая команда владельцу (пример):

```bash
git add \
  SECURITY.md docs/API.md docs/design/ADR_TSA_READ_PATH_VERIFICATION.md \
  migrations/20260725000000_tsa_token_verification_cache.sql \
  src/api/v1/proof_state.rs src/tsa/lib.rs src/tsa/types.rs src/tsa/read_verify.rs \
  tests/tsa_read_verify.rs tests/v1_proof.rs \
  vendor/notary-tsa/src/lib.rs vendor/notary-tsa/src/openssl_provider.rs
# НЕ добавлять: .gitignore, docs/audits/STAGE*.md (и при желании этот отчёт — отдельно)
```

## Изменения вне scope (остались в дереве)

| Путь | Решение владельца |
|---|---|
| `.gitignore` (`.local-backups/`) | отдельный chore-коммит или оставить локально |
| `docs/audits/STAGE1_STATUS.md` | docs-коммит аудитов |
| `docs/audits/STAGE9_STATUS.md` | docs-коммит аудитов |
| `docs/audits/TSA_HARDENING_COMPLETION.md` | опционально вместе с аудитами / отдельно |

Агент эти файлы **не откатывал** и **не включал** в список для TSA-коммита.
