# EVIDENT-LEDGER CLI PRODUCT LAYER — TЗ и план работ

## 1. Цель

Добавить CLI-утилиту `evident`, которая является единственной точкой входа для пользователя и покрывает три сценария:

- `evident hash <file>`
- `evident commit <file> --chain <id>`
- `evident verify <proof.json>`

## 2. Ограничения

Следующие ограничения нельзя менять в рамках текущего этапа:

- файл не хранится;
- используется только SHA256;
- сервер уже существует и не переписывается;
- TSA уже существует и не переписывается;
- verifier уже существует и используется как есть;
- local audit не добавляется на этом этапе.

## 3. Целевая архитектура

```text
FILE
 ↓
CLI (evident)
 ↓
SERVER (Axum ledger)
 ↓
PROOF (json)
 ↓
VERIFIER (CLI)
```

## 4. Что добавляется в репозиторий

### 4.1 CLI binary

- новый бинарник: [src/bin/evident.rs](../src/bin/evident.rs)

### 4.2 Команды CLI

#### hash

- читает файл;
- считает SHA256;
- печатает hash в stdout.

#### commit

- считает SHA256 файла;
- отправляет запрос на существующий endpoint `/events`;
- получает ответ сервера;
- сохраняет локальный proof-файл.

#### verify

- читает proof.json;
- вызывает существующий verifier workflow;
- выводит результат OK / FAIL.

## 5. Текущий контракт сервера

На текущем состоянии репозитория сервер ожидает POST `/events` с payload вида:

```json
{
  "file_hash": "sha256",
  "chain_id": "uuid",
  "parent_event_id": "uuid",
  "idempotency_key": "uuid",
  "signature": ""
}
```

Сервер возвращает JSON вида:

```json
{
  "event_id": "uuid",
  "chain_id": "uuid",
  "head_event_id": "uuid",
  "cached": false
}
```

Важно: для первого события в цепочке сервер ожидает `parent_event_id = 00000000-0000-0000-0000-000000000000`.

## 6. Текущий контракт verifier

Существующий verifier-CLI ожидает proof.json со структурой:

```json
{
  "chain_id": "...",
  "head_event_id": "...",
  "proof": {
    "root": "...",
    "chain_head": "...",
    "signature": "...",
    "public_key": "...",
    "leaves_count": 1
  },
  "events": [
    {
      "sequence": 1,
      "event_id": "...",
      "parent_event_id": "...",
      "file_hash": "..."
    }
  ]
}
```

## 7. План работ

### Шаг 1 — CLI каркас

- добавить бинари и структуру команд;
- реализовать парсинг `hash`, `commit`, `verify`.

### Шаг 2 — HASH

- прочитать файл;
- вычислить SHA256;
- вывести результат.

### Шаг 3 — COMMIT

- вычислить SHA256 файла;
- отправить POST-запрос на сервер;
- сохранить результат в локальный proof.json;
- корректно обрабатывать первый event в цепочке через `parent_event_id = nil`.

### Шаг 4 — VERIFY

- загрузить proof.json;
- вызвать существующий verifier workflow;
- вывести понятный результат пользователю.

### Шаг 5 — Проверка end-to-end

- запустить сервер;
- выполнить `evident hash`;
- выполнить `evident commit`;
- проверить, что создан proof.json;
- выполнить `evident verify`.

## 8. Definition of Done

CLI считается готовым, если следующие сценарии выполняются:

```bash
evident hash file.txt
evident commit file.txt --chain <chain-id>
evident verify proof.json
```

и работают через текущий сервер и existing verifier без изменения core-логики.

## 9. Текущее состояние репозитория

На данный момент в репозитории уже есть:

- серверный endpoint для событий: [src/api/events.rs](../src/api/events.rs)
- бизнес-логика submission: [src/service/ledger.rs](../src/service/ledger.rs)
- модель запроса: [src/models/event.rs](../src/models/event.rs)
- verifier CLI: [src/bin/verify.rs](../src/bin/verify.rs)
- новый CLI binary: [src/bin/evident.rs](../src/bin/evident.rs)

Дальнейшая работа должна быть направлена на интеграцию CLI с этими существующими компонентами, а не на создание новых абстракций или изменение core-функциональности.
