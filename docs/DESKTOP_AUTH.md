# Desktop Authentication Bridge (Stage 13.4)

Desktop GUI no longer requires pasting an API key.

## Flow

1. User signs in on the web dashboard (cookie session).
2. GUI opens `/dashboard/desktop/connect` in the browser.
3. User confirms → server creates a desktop token and redirects to a localhost callback.
4. GUI stores the plaintext token in the macOS Keychain (`com.evidentledger.desktop`).
5. API calls use `Authorization: Bearer desktop_…`.

API keys (`X-API-Key`) remain for CLI, integrations, and automation.

## Endpoints

| Method | Path | Auth | Purpose |
|--------|------|------|---------|
| `GET` | `/dashboard/desktop/connect` | session | Confirm page |
| `POST` | `/dashboard/desktop/connect` | session | Create token (JSON) |
| `POST` | `/dashboard/desktop/connect/confirm` | session | Form → redirect with token |
| `POST` | `/dashboard/desktop/tokens/:id/revoke` | session | Revoke |
| `GET` | `/v1/me` | Bearer or API key | Identity for GUI |

Tokens are stored as SHA-256 hashes only. Default TTL: 30 days.
