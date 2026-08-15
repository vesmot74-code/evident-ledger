#!/usr/bin/env bash
# Evident Ledger — local CLI entrypoint.
# Does not source `.env` or export secrets; the binary / dotenvy load config.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

if [[ ! -f .env ]]; then
  echo "ERROR: .env not found."
  echo "Create local configuration from .env.example:"
  echo "cp .env.example .env"
  exit 1
fi

exec cargo run --bin evident -- "$@"
