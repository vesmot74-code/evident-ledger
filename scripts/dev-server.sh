#!/usr/bin/env bash
# Evident Ledger — local dev server entrypoint.
# Loads no secrets: Rust/dotenvy reads `.env`. This script only orchestrates
# Docker PostgreSQL readiness and `cargo run --bin evident-ledger`.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

die() {
  echo "ERROR: $*" >&2
  exit 1
}

require_env_file() {
  if [[ ! -f .env ]]; then
    echo "ERROR: .env not found."
    echo "Create local configuration from .env.example:"
    echo "cp .env.example .env"
    exit 1
  fi
}

require_compose_v2() {
  [[ -f docker-compose.yml ]] || die "docker-compose.yml missing"
  if ! docker compose version >/dev/null 2>&1; then
    echo "ERROR: Docker Compose v2 (\`docker compose\`) is required."
    exit 1
  fi
}

# Discover the Postgres service + credentials from the resolved compose config.
# Prints: SERVICE_NAME USER DATABASE
discover_db_contract() {
  python3 - <<'PY'
import re
import subprocess
import sys

try:
    cfg = subprocess.check_output(
        ["docker", "compose", "config"],
        text=True,
        stderr=subprocess.DEVNULL,
    )
except subprocess.CalledProcessError as e:
    print(f"failed to run docker compose config: {e}", file=sys.stderr)
    sys.exit(1)

# Split into top-level service blocks under `services:` (2-space indent).
services: dict[str, str] = {}
current = None
body_lines: list[str] = []
in_services = False

def flush():
    global current, body_lines
    if current is not None:
        services[current] = "\n".join(body_lines)
    current = None
    body_lines = []

for line in cfg.splitlines():
    if line.strip() == "services:":
        in_services = True
        flush()
        continue
    if not in_services:
        continue
    # Next top-level key ends services
    if line and not line.startswith(" ") and not line.startswith("\t"):
        break
    m = re.match(r"^  ([^ #:]+):\s*$", line)
    if m:
        flush()
        current = m.group(1)
        continue
    if current is not None:
        body_lines.append(line)
flush()

chosen = None
for name, body in services.items():
    if re.search(r"(?i)image:\s*.*postgres", body) or "POSTGRES_USER" in body:
        chosen = (name, body)
        break

if chosen is None:
    print("no PostgreSQL service found in docker-compose.yml", file=sys.stderr)
    sys.exit(1)

name, body = chosen
user_m = re.search(r"POSTGRES_USER:\s*(\S+)", body)
db_m = re.search(r"POSTGRES_DB:\s*(\S+)", body)
if not user_m or not db_m:
    print(f"service {name!r} missing POSTGRES_USER/POSTGRES_DB", file=sys.stderr)
    sys.exit(1)

print(f"{name} {user_m.group(1)} {db_m.group(1)}")
PY
}

env_kv() {
  # Read a single KEY=value from .env without sourcing (no export, no eval of values).
  local key="$1"
  local line
  line="$(grep -E "^${key}=" .env 2>/dev/null | tail -n1 || true)"
  if [[ -z "$line" ]]; then
    echo ""
    return 0
  fi
  echo "${line#"${key}="}"
}

paddle_summary() {
  local raw
  raw="$(env_kv PADDLE_ENABLED)"
  raw="$(echo "$raw" | tr -d '"' | tr -d "'" | tr '[:upper:]' '[:lower:]' | xargs || true)"
  case "$raw" in
    false|0|no|off) echo "disabled" ;;
    true|1|yes|on) echo "enabled" ;;
    "") echo "default (see application)" ;;
    *) echo "configured" ;;
  esac
}

sqlx_offline_summary() {
  if [[ -f .cargo/config.toml ]] && grep -qE 'SQLX_OFFLINE\s*=\s*"true"' .cargo/config.toml; then
    echo "offline"
  else
    echo "not forced offline via .cargo/config.toml"
  fi
}

service_running() {
  local svc="$1"
  docker compose ps --status running --services 2>/dev/null | grep -qx "$svc"
}

wait_pg_ready() {
  local svc="$1" user="$2" db="$3"
  local i
  for i in $(seq 1 60); do
    if docker compose exec -T "$svc" pg_isready -U "$user" -d "$db" >/dev/null 2>&1; then
      return 0
    fi
    sleep 1
  done
  die "PostgreSQL service '$svc' did not become ready within 60s"
}

verify_db_identity() {
  local svc="$1" user="$2" db="$3"
  local row
  row="$(docker compose exec -T "$svc" \
    psql -U "$user" -d "$db" -v ON_ERROR_STOP=1 -tAc \
    "SELECT current_database() || '|' || current_user;" 2>/dev/null | tr -d '[:space:]')"
  [[ -n "$row" ]] || die "could not query current_database()/current_user via docker compose exec"
  local got_db got_user
  got_db="${row%%|*}"
  got_user="${row##*|}"
  [[ "$got_db" == "$db" ]] || die "current_database()=$got_db, expected $db"
  [[ "$got_user" == "$user" ]] || die "current_user=$got_user, expected $user"
  echo "PostgreSQL identity: database=$got_db user=$got_user"
}

maybe_migrate_info() {
  # Diagnostic only — never apply migrations.
  if ! command -v cargo >/dev/null 2>&1; then
    echo "WARN: cargo not found; skipping sqlx migrate info"
    return 0
  fi
  if cargo sqlx migrate info >/tmp/evident-migrate-info.txt 2>/tmp/evident-migrate-info.err; then
    echo "SQLx migrate info: OK (schema not modified)"
  else
    echo "ERROR: cargo sqlx migrate info failed (schema was not modified)."
    echo "----- stderr -----"
    cat /tmp/evident-migrate-info.err >&2 || true
    echo "Fix migration / DB connectivity manually, then retry."
    echo "This script does not run migrations."
    exit 1
  fi
}

main() {
  require_env_file
  require_compose_v2

  local contract db_service db_user db_name
  contract="$(discover_db_contract)"
  db_service="$(echo "$contract" | awk '{print $1}')"
  db_user="$(echo "$contract" | awk '{print $2}')"
  db_name="$(echo "$contract" | awk '{print $3}')"

  echo "=== Evident Ledger Dev Server ==="
  echo "Project: $ROOT_DIR"
  echo "Database: configured (.env → dotenvy)"
  echo "Docker DB service: $db_service (user=$db_user db=$db_name)"

  if ! service_running "$db_service"; then
    echo "PostgreSQL: starting ($db_service)..."
    docker compose up -d "$db_service"
  else
    echo "PostgreSQL: already running ($db_service)"
  fi

  wait_pg_ready "$db_service" "$db_user" "$db_name"
  echo "PostgreSQL: ready"
  verify_db_identity "$db_service" "$db_user" "$db_name"

  echo "SQLx: $(sqlx_offline_summary)"
  echo "Paddle: $(paddle_summary)"
  maybe_migrate_info

  echo "Server: http://127.0.0.1:3000"
  echo "Starting evident-ledger..."
  echo

  exec cargo run --bin evident-ledger
}

main "$@"
