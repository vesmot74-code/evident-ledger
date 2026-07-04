# SYSTEM_CONTRACT.md — Evident Ledger (актуальное состояние)

Deterministic verifiable event ledger with cryptographic proofs and offline verification.

**Статус:** это описание фактической реализации на текущий момент, не целевой архитектуры. Расхождения между компонентами отмечены явно как известный технический долг.

---

## 1. SYSTEM MODEL

- каждое действие — неизменяемое событие (event)
- события образуют hash-linked цепочку
- доверие строится на криптографическом доказательстве (merkle root + подпись сервера), backend возвращает verdict, но GUI дополнительно перепроверяет локальные файлы на диске (см. п.5)
- офлайн-проверка возможна через `evident-verify`

---

## 2. STORAGE MODEL — ⚠️ ИЗВЕСТНОЕ РАСХОЖДЕНИЕ (не архитектурное решение)

В системе сейчас существуют **два независимых, не связанных друг с другом хранилища**:

### 2.1 GUI storage (`evident-gui`)

```text
~/Evident Projects/<project_name>/
  originals/
  proofs/
  Аудит/
    audit.jsonl
```

Используется приложением `evident-gui`. Каждый проект имеет собственный `chain_id` (UUID v4), сохранённый в `project.json`.

### 2.2 CLI storage (`evident`)

```text
~/.evident/
  identity.key
  identity.pub
  events.jsonl
  proofs/<chain_id>/
```

Используется бинарником `evident` (CLI): `evident init`, `evident commit`, `evident report generate`, `evident status`.

### ⚠️ Технический долг

Эти два хранилища **не синхронизированы и не пересекаются**: файл, закоммиченный через GUI, не виден CLI-командам, и наоборот. Это исторический артефакт параллельного развития GUI и CLI, а не осознанное архитектурное решение. Унификация в единое хранилище — открытая задача на будущее.

---

## 3. CORE PIPELINE

```text
file → SHA256 → event → chain → proof → verify → report
```

Общий пайплайн одинаков для GUI и CLI, но с разными точками входа для storage (см. п.2).

---

## 4. VERIFICATION MODEL

### 4.1 Backend verification (`GET /verify/{chain_id}`)

Backend пересчитывает merkle root по всем событиям цепочки, проверяет подпись, возвращает `{valid, blocks, head_event_id, errors}`.

### 4.2 Local file integrity check (GUI only)

GUI дополнительно сверяет SHA-256 файлов в `originals/` с `file_hash` из ответа backend. Итоговый статус в GUI — комбинация двух независимых осей:

- `backend_valid` — криптографическая целостность цепочки (источник истины)
- `local_integrity_ok` — совпадение локальной копии файла с зафиксированным хэшем (клиентская проверка, обнаруживает локальную подмену файла на диске)

CLI (`evident-verify`) выполняет офлайн-проверку по файлу `proof.json`: merkle root, подпись (pinned server key через `~/.evident/server_identity.pub`), опционально сверку с оригиналом файла (`Original: OK` / `Original: MISSING or MISMATCH`).

---

## 5. AUDIT MODEL

`audit.jsonl` (GUI) — append-only, каждая запись:

```json
{
  "event_id": "...",
  "chain_id": "...",
  "file_hash": "...",
  "sequence": 1,
  "parent_event_id": "...",
  "created_at": "...",
  "kind": { "Anchored": { "server_event_id": "...", "proof": {...} } }
}
```

Каждая фиксация файла создаёт две записи: `Submitted` (sequence: null, до ответа сервера) и `Anchored` (sequence: реальное значение от backend, после подтверждения).

---

## 6. ORIGINALS NAMING

```text
originals/{sequence:04}_{filename}
```

Пример: `0001_document.rtf`, `0002_report.pdf`.

---

## 7. TSA (RFC 3161)

Через vendored крейт `notary-tsa` (внешний провайдер FreeTSA). TSA-данные опциональны в отчёте: если `timestamp`/`serial`/`token_bytes` неполны — PDF-сертификат явно показывает "Статус TSA: Не подтверждено", не выдаёт ложное "Подтверждено" с нулевой датой.

---

## 8. REPORT GENERATION (`evident report generate`)

Требует обязательного наличия `file_hash` и `chain_id` в proof.json — при отсутствии команда завершается ошибкой `incomplete proof: missing <field>`, PDF не генерируется. TSA-поля остаются опциональными.

---

## 9. DEPENDENCIES

- `notary-tsa`, `notary-pdf` — vendored в `vendor/` (копии крейтов из отдельного репозитория `notary-core`, скопированы для самодостаточности сборки)
- PostgreSQL — требуется для запуска сервера (`evident-ledger` bin), не требуется для сборки (офлайн sqlx-кэш в `.sqlx/`)

---

## 10. KNOWN LIMITATIONS

- GUI и CLI используют разные, не связанные хранилища (см. п.2)
- ZIP-экспорт в GUI: кнопка присутствует, функциональность не реализована
- TSA зависит от доступности внешнего провайдера (FreeTSA)
- Сервер требует запущенный PostgreSQL

---

## 11. WHAT IS ACTUALLY LOCKED (проверено тестами)

- audit.jsonl append-only механизм — `cargo test` покрывает
- merkle root recompute + подпись — `cargo test` покрывает (`tests/verifier.rs`, 4 сценария)
- sequence monotonicity в verify_project (CLI) — покрыто
- local file integrity check (GUI) — проверено вручную (регрессионный сценарий с подменой файла)

Всё остальное — рабочая, но не formально закреплённая тестами область; при рефакторинге проверяйте вручную по сценариям выше.
