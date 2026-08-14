# GUI PDF/ZIP Buttons Cleanup

**Date:** 2026-08-14  
**Branch:** `gui/pdf-buttons-cleanup`  
**Scope:** UI-only refactor of Result screen and Evidence Verification Dashboard. No generators, crypto, API, storage, or verification engine changes.

---

## Goal

Simplify PDF/ZIP artifact actions:

- One clear primary certificate action on the commit Result screen
- Clearer labels on the verification dashboard
- No duplicate “Generate Certificate PDF” on the dashboard
- No new PDF/ZIP formats

---

## Before → After

### Result screen (`Screen::Result`)

| Before | After |
| --- | --- |
| Проверить | Создать сертификат PDF (**primary**) |
| Открыть PDF → `registration_snapshot.pdf` | *(removed from UI)* |
| Generate Certificate PDF | *(renamed / primary)* |
| Вся цепочка аудита | Проверить |
| Сохранить копию файла | Вся цепочка аудита |
| | Сохранить копию файла |

Final order:

1. **Создать сертификат PDF** (primary, accent fill)
2. Проверить
3. Вся цепочка аудита
4. Сохранить копию файла

### Verification Dashboard (`Screen::VerifyResult`)

| Before | After |
| --- | --- |
| Event row: `PDF` | Event row: **Снимок регистрации** (same `export_event_pdf` logic) |
| Event row: `ZIP (скоро)` | Unchanged |
| Generate Certificate PDF | **Removed** (certificate only on Result) |
| Скачать заключение (PDF) | Unchanged → `chain_verification_report.pdf` |
| Скачать проект (ZIP) + checkbox | Unchanged |
| Назад | Unchanged |

Bottom block:

1. Скачать заключение (PDF)
2. Скачать проект (ZIP)
3. ☑ Включить файлы в архив
4. Назад

### Localization

The bug was **not** wrong RU/EN translation strings.

**Root cause:** in `verify_project`, the offline blurb was built once as:

```rust
self.verification_report = self.tr(ru, en).to_string();
```

Then `Screen::VerifyResult` showed the frozen `String` via `ui.label(&self.verification_report)`. Switching RU/EN updated `self.lang` but did not rebuild that field, so the language from the moment of verification stayed on screen (default UI language is `En`).

**Fix:** keep `verification_report` non-empty only as a presence flag; at paint time render with the current language:

```rust
if !self.verification_report.is_empty() {
    ui.label(self.tr(ru, en));
}
```

Canonical copy:

| Lang | Text |
| --- | --- |
| RU | Локальная проверка: цепочка событий, файлы на диске и криптографическая подпись. |
| EN | Local check: event chain, files on disk, and cryptographic signature. |

---

## Changed files

| File | Change |
| --- | --- |
| `evident-gui-app/src/main.rs` | Button labels, order, removals, primary styling; Local check paint-time `tr()` |
| `docs/audits/gui-pdf-buttons-cleanup.md` | This audit |

**Not changed:** `evident-report`, `file_certificate_pdf`, ZIP helpers, crypto, API, DB, account/backup screens.

`build_registration_proof_data` retained (unused by Result UI after removing Открыть PDF). `generate_registration_snapshot` remains in `evident-report` (import dropped from GUI only).

---

## Screenshots before/after

Screenshots were not captured in this agent run. Visual check checklist:

- [ ] Result: no «Открыть PDF»; primary «Создать сертификат PDF» first
- [ ] Verify dashboard: no «Generate Certificate PDF»; event button «Снимок регистрации»
- [ ] Bottom: заключение PDF + ZIP + checkbox + Назад only

---

## Known follow-up

| Field | Risk |
| --- | --- |
| `self.status` | Некоторые сообщения могут сохраняться после `tr().to_string()` и не обновляться при смене языка. Проверить отдельным проходом. |
| `self.error_message` | Проверить, не сохраняются ли локализованные строки вместо ключа/рендера через текущий язык. |
| `self.verification_report` | Исправлено в этом cleanup: отображается через `self.tr()` в момент рендера. |
