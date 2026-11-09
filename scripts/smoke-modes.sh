#!/usr/bin/env bash
# UWAGA: skrypt testowy. Używa wyłącznie danych testowych, losowo
# generowanych kont i izolowanej bazy w /tmp/opencode. Nie uruchamiaj go
# na prawdziwej instancji i nigdy nie używaj tych danych logowania w
# produkcji.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

BIN="$ROOT/target/debug/zsl_order"
BASE="/tmp/opencode/zsl-section1"
FAILS=0

pass() { printf 'PASS  %s\n' "$*"; }
fail() { printf 'FAIL  %s\n' "$*"; FAILS=$((FAILS + 1)); }

cleanup() {
  for pid in "${PIDS[@]:-}"; do
    kill "$pid" 2>/dev/null || true
    wait "$pid" 2>/dev/null || true
  done
}
PIDS=()
trap cleanup EXIT

start_instance() {
  local dir=$1
  (cd "$dir" && exec env RUST_LOG=warn "$BIN" >server.log 2>&1) &
  PIDS+=("$!")
}

wait_for_admin() {
  local dir=$1
  for _ in $(seq 1 100); do
    [ -f "$dir/admin_start.txt" ] && return 0
    sleep 0.1
  done
  return 1
}

credentials() {
  local file=$1
  LOGIN=$(sed -n 's/^Login:  //p' "$file" | head -n1)
  PASSWORD=$(sed -n 's/^Hasło:  //p' "$file" | head -n1)
}

wait_for_http() {
  local port=$1 host=$2
  for _ in $(seq 1 150); do
    if curl -s --noproxy '*' -o /dev/null \
      -H "Host: $host" "http://127.0.0.1:$port/api/public/info"; then
      return 0
    fi
    sleep 0.2
  done
  return 1
}

rm -rf "$BASE"
mkdir -p "$BASE/local" "$BASE/tunnel" "$BASE/badbind" "$BASE/nourl"

# ---------------------------------------------------------------- local mode
cat >"$BASE/local/config.json" <<'JSON'
{"db_url":"sqlite://baza.db","max_connections":4,"session_hours":1}
JSON
cat >"$BASE/local/.env" <<'ENV'
DEPLOY_MODE=local
BIND_ADDR=127.0.0.1:38181
TRUSTED_PROXIES=127.0.0.1,::1
PUBLIC_URL=http://127.0.0.1:38181
COOKIE_SECURE=false
ENV

start_instance "$BASE/local"
wait_for_admin "$BASE/local" || fail "local: brak admin_start.txt"
wait_for_http 38181 127.0.0.1:38181 || fail "local: serwer nie odpowiada"
credentials "$BASE/local/admin_start.txt"

INFO=$(curl -s -o "$BASE/local/info.json" -w '%{http_code}' \
  -H 'Host: 127.0.0.1:38181' http://127.0.0.1:38181/api/public/info)
[ "$INFO" = "200" ] && pass "local: public info 200" || fail "local: public info $INFO"

HEADERS=$(curl -s -D - -o /dev/null \
  -H 'Host: 127.0.0.1:38181' http://127.0.0.1:38181/api/public/info)
printf '%s' "$HEADERS" | grep -qi '^strict-transport-security:' \
  && fail "local: HSTS nie powinien być wysyłany" \
  || pass "local: brak HSTS"

BAD_ORIGIN=$(curl -s -o /dev/null -w '%{http_code}' -X POST \
  -H 'Host: 127.0.0.1:38181' \
  -H 'Origin: http://evil.example' \
  -H 'Content-Type: application/json' \
  -d '{"login":"x","password":"y"}' \
  http://127.0.0.1:38181/api/auth/login)
[ "$BAD_ORIGIN" = "403" ] && pass "local: obcy Origin 403" || fail "local: obcy Origin $BAD_ORIGIN"

LOGIN_HEADERS=$(curl -s -D - -o "$BASE/local/login.json" -X POST \
  -H 'Host: 127.0.0.1:38181' \
  -H 'Origin: http://127.0.0.1:38181' \
  -H 'Content-Type: application/json' \
  -d "{\"login\":\"$LOGIN\",\"password\":\"$PASSWORD\"}" \
  http://127.0.0.1:38181/api/auth/login)
printf '%s' "$LOGIN_HEADERS" | grep -q 'set-cookie:.*Secure' \
  && fail "local: ciasteczko nie powinno być Secure przy COOKIE_SECURE=false" \
  || pass "local: ciasteczko bez Secure"

# ---------------------------------------------------------------- tunnel mode
cat >"$BASE/tunnel/config.json" <<'JSON'
{"db_url":"sqlite://baza.db","max_connections":4,"session_hours":1}
JSON
cat >"$BASE/tunnel/.env" <<'ENV'
DEPLOY_MODE=tunnel
BIND_ADDR=127.0.0.1:38182
TRUSTED_PROXIES=127.0.0.1
PUBLIC_URL=https://zsl.example.com
COOKIE_SECURE=false
ENV

start_instance "$BASE/tunnel"
wait_for_admin "$BASE/tunnel" || fail "tunnel: brak admin_start.txt"
wait_for_http 38182 zsl.example.com || fail "tunnel: serwer nie odpowiada"
credentials "$BASE/tunnel/admin_start.txt"

PUBLIC=$(curl -s -o /dev/null -w '%{http_code}' \
  -H 'Host: zsl.example.com' http://127.0.0.1:38182/api/public/info)
[ "$PUBLIC" = "200" ] && pass "tunnel: publiczny Host 200" || fail "tunnel: publiczny Host $PUBLIC"

EVIL_HOST=$(curl -s -o /dev/null -w '%{http_code}' \
  -H 'Host: evil.example' http://127.0.0.1:38182/api/public/info)
[ "$EVIL_HOST" = "403" ] && pass "tunnel: obcy Host 403" || fail "tunnel: obcy Host $EVIL_HOST"

LOOPBACK=$(curl -s -o /dev/null -w '%{http_code}' \
  -H 'Host: 127.0.0.1:38182' http://127.0.0.1:38182/api/public/info)
[ "$LOOPBACK" = "200" ] && pass "tunnel: loopback Host 200" || fail "tunnel: loopback Host $LOOPBACK"

HEADERS=$(curl -s -D - -o /dev/null \
  -H 'Host: zsl.example.com' http://127.0.0.1:38182/api/public/info)
HSTS=$(printf '%s' "$HEADERS" | tr -d '\r' | sed -n 's/^[Ss]trict-[Tt]ransport-[Ss]ecurity: //p')
[ "$HSTS" = "max-age=31536000" ] \
  && pass "tunnel: HSTS bez includeSubDomains/preload" \
  || fail "tunnel: nieprawidłowy lub brak HSTS ($HSTS)"

LOGIN_HEADERS=$(curl -s -D - -o "$BASE/tunnel/login.json" -X POST \
  -H 'Host: zsl.example.com' \
  -H 'Origin: https://zsl.example.com' \
  -H 'Content-Type: application/json' \
  -d "{\"login\":\"$LOGIN\",\"password\":\"$PASSWORD\"}" \
  http://127.0.0.1:38182/api/auth/login)
printf '%s' "$LOGIN_HEADERS" | grep -q 'set-cookie:.*Secure' \
  && pass "tunnel: ciasteczko Secure wymuszone" \
  || fail "tunnel: ciasteczko bez Secure"

TUNNEL_BAD_ORIGIN=$(curl -s -o /dev/null -w '%{http_code}' -X POST \
  -H 'Host: zsl.example.com' \
  -H 'Origin: http://127.0.0.1:38182' \
  -H 'Content-Type: application/json' \
  -d '{"login":"x","password":"y"}' \
  http://127.0.0.1:38182/api/auth/login)
[ "$TUNNEL_BAD_ORIGIN" = "403" ] && pass "tunnel: lokalny Origin 403" || fail "tunnel: lokalny Origin $TUNNEL_BAD_ORIGIN"

# ---------------------------------------------------------------- config errors
(cd "$BASE/local" && RUST_LOG=warn "$BIN" >second.log 2>&1)
STATUS=$?
[ "$STATUS" -ne 0 ] && pass "local: zajęty port kończy start" || fail "local: fallback przy zajętym porcie"
grep -qi 'nasłuchu\|Address already in use' "$BASE/local/second.log" \
  && pass "local: czytelny błąd zajętego portu" \
  || fail "local: brak błędu zajętego portu"

cat >"$BASE/badbind/config.json" <<'JSON'
{"db_url":"sqlite://baza.db","max_connections":4,"session_hours":1}
JSON
cat >"$BASE/badbind/.env" <<'ENV'
DEPLOY_MODE=tunnel
BIND_ADDR=0.0.0.0:38183
TRUSTED_PROXIES=127.0.0.1
PUBLIC_URL=https://zsl.example.com
COOKIE_SECURE=auto
ENV
(cd "$BASE/badbind" && RUST_LOG=warn "$BIN" >server.log 2>&1)
STATUS=$?
[ "$STATUS" -ne 0 ] && pass "tunnel: odrzucono BIND_ADDR 0.0.0.0" || fail "tunnel: przyjęto BIND_ADDR 0.0.0.0"
grep -qi 'loopback' "$BASE/badbind/server.log" && pass "tunnel: czytelny błąd loopback" || fail "tunnel: brak błędu loopback"

cat >"$BASE/nourl/config.json" <<'JSON'
{"db_url":"sqlite://baza.db","max_connections":4,"session_hours":1}
JSON
cat >"$BASE/nourl/.env" <<'ENV'
DEPLOY_MODE=tunnel
BIND_ADDR=127.0.0.1:38184
TRUSTED_PROXIES=127.0.0.1
COOKIE_SECURE=auto
ENV
(cd "$BASE/nourl" && RUST_LOG=warn "$BIN" >server.log 2>&1)
STATUS=$?
[ "$STATUS" -ne 0 ] && pass "tunnel: brak PUBLIC_URL kończy start" || fail "tunnel: start bez PUBLIC_URL"
grep -qi 'PUBLIC_URL' "$BASE/nourl/server.log" && pass "tunnel: czytelny błąd PUBLIC_URL" || fail "tunnel: brak błędu PUBLIC_URL"

printf '\nWynik: %d błędów\n' "$FAILS"
exit "$FAILS"
