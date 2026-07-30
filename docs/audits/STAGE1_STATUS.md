# Stage 1 Status Audit — 2026-07-30

## Summary

По чеклисту ТЗ (шаги 0–6) Stage 1 **фактически завершён**: DB policy, модуль `/v1`, `ApiError` + `request_id`, auth, ownership (404), idempotency storage + integration, derived `proof_status` — всё есть в коде и покрыто тестами. Реализация давно ушла дальше Stage 1 (identity, billing, desktop auth, public proof и т.д.). Единственный явный остаток в исходном scope Stage 1 — заглушка `GET /v1/account/capabilities` (`NotImplemented`). Разработка Stage 1 foundation зафиксирована коммитом `6648656` (2026-07-17); текущий HEAD — `cfe1c83` (landing swap), с незакоммиченной работой по TSA read-path verification (не относится к Stage 1).

## Step-by-step status

| Шаг | Статус | Комментарий |
|---|---|---|
| 0. DB policy | done | `docs/DB_POLICY.md` существует; текст совпадает с согласованным: development/test data, schema-only migration, data migration not required, recreate DB before v1 rollout. |
| 1. v1 module structure | done | Есть `src/api/v1/` с `mod.rs`, `events.rs`, `proof.rs`, `verify.rs`, `account.rs`, `errors.rs` (+ много модулей сверх плана). `/v1` подключён в `src/main.rs` (`nest("/v1", …)`). Legacy `/events`, `/verify`, `/backup` остаются отдельными nest’ами рядом с `/v1`. Endpoints `/v1/events`, `/v1/proof/{id}`, `/v1/verify/{id}` — **реализованы**, не заглушки; `/v1/account/capabilities` — stub (`ApiError::NotImplemented`). |
| 2. ApiError + request_id | done | `enum ApiError` содержит `Unauthorized/Forbidden/NotFound/Conflict/InvalidRequest/Internal` и расширен позже (proof/billing/identity и т.д.). `IntoResponse` → `{ "error": { "code", "message", "request_id" } }`. `request_id_layer` навешан на v1 router в `mod.rs`. Unit-тесты сериализации в `errors.rs` (в т.ч. `unauthorized_serializes_with_request_id`) — 3/3 ok. |
| 3. Auth v1 | done | `V1Auth` + `v1_auth_middleware` через `resolve_authed_account` (`X-API-KEY` / desktop Bearer). Middleware на всём `/v1` router; handlers также принимают `V1Auth`. 401 envelope подтверждён тестом `revoke_without_api_key_returns_unauthorized` (`status=401`, `error.code=unauthorized`) и unit-тестом сериализации `Unauthorized`. |
| 4. Ownership guard | done | `verify_event_access(pool, account_id, event_id)` в `event_access.rs` (service-layer, не middleware). Применяется в `proof.rs` и `verify.rs`. Чужой/отсутствующий event → **`404 NotFound`** (совпадает с согласованным решением и `docs/API.md` §1). Lib-тесты ownership: 3/3 ok. |
| 5a. Idempotency storage | done | Миграция `migrations/20260717000000_idempotency_records.sql`: поля + `UNIQUE(account_id, idempotency_key)` (`uniq_idempotency_account_key`). Применена к локальной БД (`\d idempotency_records` через `.env` — таблица на месте). `IdempotencyRepository` с `find`/`insert`; Postgres helpers `find_active_in_tx`/`insert_in_tx`. `request_hash` = `canonical_json_sha256` (canonical JSON + SHA-256). TTL: константа `IDEMPOTENCY_TTL_HOURS = 24` в `idempotency/mod.rs`. |
| 5b. Idempotency integration | done | Подключено в `submit_v1_event` / `POST /v1/events`: lookup → replay `200` с `response_json`; hash mismatch → `Conflict` (409); insert event + idempotency record в **одной** транзакции, затем `commit`. Интеграционный тест `tests/v1_events_idempotency.rs` описывает replay+conflict (требует live server `:3000` — при аудите server недоступен, тест упал на connect refused). |
| 6. proof_status | done | `derive_proof_status()` в `proof_status.rs`; enum `pending` / `anchored` / `failed` совпадает с `docs/API.md`. Колонки `events.proof_status` **нет** (проверено в `information_schema` + migrations: `proof_status` только у public-registry таблиц). Lib-тесты proof_status: 9/9 ok. |

## Расхождения с ТЗ

1. **`GET /v1/account/capabilities`** — всё ещё stub (`NotImplemented`), хотя маршрут и auth есть. Это единственный незакрытый deliverable из исходного набора Stage 1 endpoints.
2. **`docs/API_IMPLEMENTATION_PLAN.md` устарел относительно кода и `docs/API.md`:** план всё ещё описывает ownership как **403**, optional `Idempotency-Key`, UPPERCASE error codes и неотмеченные чеклисты Stage 1–3. Фактически: ownership **404**, `Idempotency-Key` **required**, codes в snake_case (`unauthorized`, `not_found`, …) — как в `API.md`. Это расхождение **документа плана**, не кода.
3. **Объём `/v1` сильно шире Stage 1** (identity keys, `/v1/me`, chain/file verify helpers, subscription middleware и т.д.) — развитие после foundation, не блокер Stage 1.
4. Комментарий в `InMemoryIdempotencyRepository` («Not connected to PostgreSQL on this step») — устаревший: production path использует Postgres `insert_in_tx` / `find_active_in_tx`.

**Не расхождение с согласованным ТЗ аудита:** чужой `event_id` → **404** (как договорились); план-док с 403 — stale.

## Build/test state

Проверено на рабочем дереве (HEAD `cfe1c83` + незакоммиченные TSA-изменения). Код **не правился** в рамках аудита.

### `cargo build`

```text
Finished `dev` profile [unoptimized + debuginfo] target(s) in 18.66s
```

Сборка успешна (много warnings, в т.ч. unused TSA writer items; future-incompat sqlx-postgres).

### `cargo test --lib`

```text
test result: ok. 146 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 30.01s
```

Точечно:

| Suite | Result |
|---|---|
| `api::v1::errors` | 3 passed |
| `api::v1::event_access` | 3 passed |
| `api::v1::proof_status` | 9 passed |
| `identity_revoke::revoke_without_api_key_returns_unauthorized` | ok (401 + envelope) |

### Live integration (нужен server `:3000`)

```text
cargo test --test v1_events_idempotency
… Connection refused (http://127.0.0.1:3000/v1/events)
test v1_idempotency_replay_and_conflict ... FAILED
```

Полный `cargo test` (все integration) **не гонялся целиком**: часть suite зависит от запущенного сервера/`DATABASE_URL`. Lib-suite на HEAD зелёный.

## Uncommitted changes

На момент аудита:

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
?? docs/design/ADR_TSA_READ_PATH_VERIFICATION.md
?? migrations/20260725000000_tsa_token_verification_cache.sql
?? src/tsa/read_verify.rs
?? tests/tsa_read_verify.rs
```

`git diff --stat` (tracked): ~9 files, +209 / −116 — незакоммиченный **TSA read-path verification** (cache columns, `read_verify`, proof_state wiring, docs). К Stage 1 foundation не относится.

Последние коммиты (`git log`): landing swap, security signature fix, desktop auth, onboarding — **нет** активных WIP-коммитов по Stage 1; foundation уже в истории (`6648656` включает Stage 1 + proof/idempotency). Расхождений «коммит говорит X, файлов нет» по Stage 1 не видно: заявленное в `6648656` присутствует и расширено последующими коммитами.

## Рекомендация по следующему шагу

1. **Считать Stage 1 (шаги 0–6) закрытым** по foundation.
2. Если нужно добить исходный Stage 1 checklist до конца — единственный подшаг: реализовать **`GET /v1/account/capabilities`** по контракту `docs/API.md` / SYSTEM_CONTRACT (сейчас `account.rs` → `NotImplemented`).
3. Параллельно (docs-only, не код Stage 1): синхронизировать или пометить устаревшим `docs/API_IMPLEMENTATION_PLAN.md` (403→404, optional→required Idempotency-Key, checkboxes), чтобы план не вводил в заблуждение.
4. Незакоммиченный TSA read-path — отдельный трек: либо закоммитить отдельно, либо отложить; к продолжению Stage 1 не привязывать.
5. Перед интеграционными проверками idempotency/v1 — поднять server на `:3000` с `DATABASE_URL` / `EVIDENT_API_KEY`.
