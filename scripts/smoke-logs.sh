#!/usr/bin/env bash
# UWAGA: skrypt testowy. Tworzy izolowaną bazę w /tmp/opencode i nie
# dotyka prawdziwych danych. Wymaga zbudowanej binarki (`cargo build`)
# oraz jq.
#
# Testuje: logi JSON na stdout (bez plików logs/), brak hasła w logach,
# zdarzenia bezpieczeństwa z IP i identyfikatorem konta, brak IP w logach
# żądań HTTP oraz brak pola login w zdarzeniach nieudanego logowania.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN="$ROOT/target/debug/zsl_order"
BASE="/tmp/opencode/zsl-logs-test"
PORT=38352
FAILS=0
SERVER_PID=""

pass() { printf 'PASS  %s\n' "$*"; }
fail() { printf 'FAIL  %s\n' "$*"; FAILS=$((FAILS + 1)); }

cleanup() {
  if [ -n "$SERVER_PID" ]; then
    kill "$SERVER_PID" 2>/dev/null || true
    wait "$SERVER_PID" 2>/dev/null || true
  fi
}
trap cleanup EXIT

[ -x "$BIN" ] || { echo "Najpierw: cargo build"; exit 1; }
command -v jq >/dev/null || { echo "Wymagany jq"; exit 1; }

rm -rf "$BASE"
mkdir -p "$BASE"
ln -s "$ROOT/frontend" "$BASE/frontend"

cat >"$BASE/config.json" <<'JSON'
{
  "db_url": "sqlite://baza.db",
  "max_connections": 4,
  "session_hours": 12,
  "login_max_failures": 3,
  "login_ip_max_failures": 10,
  "login_window_minutes": 10,
  "login_lockout_minutes": 15
}
JSON

cat >"$BASE/.env" <<ENV
DEPLOY_MODE=local
BIND_ADDR=127.0.0.1:$PORT
TRUSTED_PROXIES=127.0.0.1,::1
COOKIE_SECURE=false
ENV

(cd "$BASE" && exec env RUST_LOG=info "$BIN" >server.log 2>&1) &
SERVER_PID=$!

READY=0
for _ in $(seq 1 150); do
  if curl -s --noproxy '*' -o /dev/null "http://127.0.0.1:$PORT/api/healthz"; then
    READY=1
    break
  fi
  sleep 0.2
done
if [ "$READY" = "1" ]; then
  pass "start: aplikacja odpowiada"
else
  fail "start: aplikacja nie odpowiada"
  tail -20 "$BASE/server.log"
  exit 1
fi

LOGIN="$(sed -n 's/^Login:  //p' "$BASE/admin_start.txt" | head -n1)"
PASSWORD="$(sed -n 's/^Hasło:  //p' "$BASE/admin_start.txt" | head -n1)"
[ -n "$LOGIN" ] && [ -n "$PASSWORD" ] \
  && pass "start: dane admina tylko w admin_start.txt" \
  || fail "start: brak danych admina w admin_start.txt"

[ ! -e "$BASE/logs" ] && pass "logi: aplikacja nie tworzy katalogu logs/" \
  || fail "logi: powstał katalog logs/"

# Każda linia stdout musi być poprawnym JSON-em.
INVALID=0
while IFS= read -r line; do
  [ -z "$line" ] && continue
  if ! printf '%s' "$line" | jq -e . >/dev/null 2>&1; then
    INVALID=$((INVALID + 1))
    printf '  nie-JSON: %s\n' "$line"
  fi
done <"$BASE/server.log"
[ "$INVALID" = "0" ] && pass "logi: wszystkie linie to JSON" \
  || fail "logi: $INVALID linii nie jest JSON-em"

if grep -Fq "$PASSWORD" "$BASE/server.log"; then
  fail "logi: hasło startowego admina wyciekło do logów"
else
  pass "logi: brak hasła startowego admina w logach"
fi

# Trzy nieudane próby, czwarta trafia na blokadę limitera.
for _ in 1 2 3 4; do
  curl -s --noproxy '*' -o /dev/null -H 'Content-Type: application/json' \
    -d "{\"login\":\"$LOGIN\",\"password\":\"zle-haslo\"}" \
    "http://127.0.0.1:$PORT/api/auth/login" || true
done

FAILED="$(jq -c 'select(.fields.event == "login_failed")' "$BASE/server.log" | tail -n1)"
if [ -n "$FAILED" ]; then
  pass "logi: zdarzenie login_failed"
  printf '%s' "$FAILED" | jq -e '.fields.ip != null' >/dev/null \
    && pass "logi: login_failed zawiera IP" \
    || fail "logi: login_failed bez IP"
  printf '%s' "$FAILED" | jq -e '.fields.account_id != null' >/dev/null \
    && pass "logi: login_failed zawiera account_id" \
    || fail "logi: login_failed bez account_id"
  printf '%s' "$FAILED" | jq -e '.fields.login == null' >/dev/null \
    && pass "logi: login_failed nie zawiera loginu" \
    || fail "logi: login_failed zawiera login"
else
  fail "logi: brak zdarzenia login_failed"
fi

LOCKED="$(jq -c 'select(.fields.event == "login_locked")' "$BASE/server.log" | tail -n1)"
if [ -n "$LOCKED" ]; then
  pass "logi: zdarzenie login_locked"
  printf '%s' "$LOCKED" | jq -e '.fields.ip != null and .fields.retry_after != null' >/dev/null \
    && pass "logi: login_locked zawiera IP i retry_after" \
    || fail "logi: login_locked bez IP/retry_after"
else
  fail "logi: brak zdarzenia login_locked"
fi

# Logi żądań nie mogą zawierać IP ani loginu.
if jq -e 'select(.fields.event == "http_request"
               and ((.fields.ip != null) or (.fields.login != null)))' \
  "$BASE/server.log" >/dev/null 2>&1; then
  fail "logi: http_request zawiera IP lub login"
else
  pass "logi: http_request bez IP i loginu"
fi

printf '\nWynik: %d błędów\n' "$FAILS"
exit "$FAILS"
