#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
# UWAGA: skrypt testowy. Używa wyłącznie danych testowych i izolowanej bazy
# w /tmp/opencode. Nie uruchamiaj go na prawdziwej instancji.
#
# Sprawdza przerwy, limity „na przerwę”, zamawianie z wyprzedzeniem oraz
# to, że stan magazynowy przyszłych dni nie jest kontrolowany.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN="$ROOT/target/debug/zsl-order"
BASE="/tmp/opencode/zsl-breaks"
TMP="$BASE/tmp"
DB="$BASE/baza.db"
HOSTADDR="127.0.0.1:38193"
ORIGIN="http://$HOSTADDR"
FAILS=0
SERVER_PID=""

pass() { printf 'PASS  %s\n' "$*"; }
fail() { printf 'FAIL  %s\n' "$*"; FAILS=$((FAILS + 1)); }

STATUS=""
BODY=""
CSRF=""

request() {
  local method=$1 path=$2 jar=$3 csrf=$4 data=$5 ctype=${6:-application/json}
  local args=(-s --noproxy '*' -X "$method" -H "Host: $HOSTADDR" -H "Origin: $ORIGIN"
    -o "$TMP/body" -w '%{http_code}')
  [ -n "$jar" ] && args+=(-b "$jar")
  [ -n "$csrf" ] && args+=(-H "X-CSRF-Token: $csrf")
  if [ -n "$data" ]; then
    args+=(-H "Content-Type: $ctype" --data-binary "$data")
  fi
  STATUS=$(curl "${args[@]}" "http://$HOSTADDR$path")
  BODY=$(cat "$TMP/body" 2>/dev/null || true)
}

login() {
  local user=$1 pw=$2 jar=$3
  STATUS=$(curl -s --noproxy '*' -c "$jar" -H "Host: $HOSTADDR" -H "Origin: $ORIGIN" \
    -H 'Content-Type: application/json' -o "$TMP/body" -w '%{http_code}' \
    -d "{\"login\":\"$user\",\"password\":\"$pw\"}" "http://$HOSTADDR/api/auth/login")
  BODY=$(cat "$TMP/body")
  CSRF=$(printf '%s' "$BODY" | jq -r '.csrf_token // empty')
}

start_server() {
  (cd "$BASE" && exec env RUST_LOG=warn "$BIN" >>server.log 2>&1) &
  SERVER_PID=$!
  for _ in $(seq 1 200); do
    if curl -s --noproxy '*' -o /dev/null -H "Host: $HOSTADDR" \
      "http://$HOSTADDR/api/healthz"; then
      return 0
    fi
    sleep 0.2
  done
  return 1
}

stop_server() {
  if [ -n "$SERVER_PID" ]; then
    kill "$SERVER_PID" 2>/dev/null || true
    wait "$SERVER_PID" 2>/dev/null || true
    SERVER_PID=""
  fi
}

cleanup() { stop_server; }
trap cleanup EXIT

db_scalar() { sqlite3 "$DB" "$1"; }

[ -x "$BIN" ] || { echo "Najpierw: cargo build"; exit 1; }
command -v sqlite3 >/dev/null || { echo "Wymagany sqlite3"; exit 1; }
command -v jq >/dev/null || { echo "Wymagany jq"; exit 1; }

rm -rf "$BASE"
mkdir -p "$BASE" "$TMP"
cat >"$BASE/config.json" <<'JSON'
{
  "db_url": "sqlite://baza.db",
  "max_connections": 4,
  "session_hours": 1,
  "login_max_failures": 10,
  "login_ip_max_failures": 300,
  "login_window_minutes": 10,
  "login_lockout_minutes": 15
}
JSON
cat >"$BASE/.env" <<'ENV'
DEPLOY_MODE=local
BIND_ADDR=127.0.0.1:38193
COOKIE_SECURE=false
ENV

start_server || { echo "Serwer nie wystartował"; exit 1; }

for _ in $(seq 1 100); do
  [ -f "$BASE/admin_start.txt" ] && break
  sleep 0.1
done
ADMIN_LOGIN=$(sed -n 's/^Login:  //p' "$BASE/admin_start.txt" | head -n1)
ADMIN_PASS=$(sed -n 's/^Hasło:  //p' "$BASE/admin_start.txt" | head -n1)

ADMIN_JAR="$TMP/admin.jar"
login "$ADMIN_LOGIN" "$ADMIN_PASS" "$ADMIN_JAR"
[ "$STATUS" = "200" ] && pass "admin: logowanie startowe" || fail "admin: logowanie $STATUS"
ADMIN_CSRF="$CSRF"
request POST /api/auth/change-password "$ADMIN_JAR" "$ADMIN_CSRF" \
  '{"current_password":"'"$ADMIN_PASS"'","new_password":"NoweAdmin#2026x"}'
[ "$STATUS" = "200" ] && pass "admin: zmiana hasła startowego" || fail "admin: zmiana hasła $STATUS"

request POST /api/admin/users/import "$ADMIN_JAR" "$ADMIN_CSRF" $'9A;1\n' "text/csv"
S1_LOGIN=$(printf '%s' "$BODY" | jq -r '.accounts[0].login')
S1_PASS=$(printf '%s' "$BODY" | jq -r '.accounts[0].password')
[ -n "$S1_LOGIN" ] && pass "import: konto ucznia 1" || fail "import: brak konta ($STATUS/$BODY)"
S1_JAR="$TMP/s1.jar"
login "$S1_LOGIN" "$S1_PASS" "$S1_JAR"
S1_CSRF="$CSRF"
request POST /api/auth/change-password "$S1_JAR" "$S1_CSRF" \
  '{"current_password":"'"$S1_PASS"'","new_password":"UczenHaslo10"}'
[ "$STATUS" = "200" ] && pass "uczeń 1: zmiana hasła startowego" || fail "uczeń 1: hasło $STATUS"

request POST /api/admin/users/generate "$ADMIN_JAR" "$ADMIN_CSRF" '{"role":"student","count":1}'
S2_LOGIN=$(printf '%s' "$BODY" | jq -r '.accounts[0].login')
S2_PASS=$(printf '%s' "$BODY" | jq -r '.accounts[0].password')
S2_JAR="$TMP/s2.jar"
login "$S2_LOGIN" "$S2_PASS" "$S2_JAR"
S2_CSRF="$CSRF"
request POST /api/auth/change-password "$S2_JAR" "$S2_CSRF" \
  '{"current_password":"'"$S2_PASS"'","new_password":"UczenHaslo11"}'
[ "$STATUS" = "200" ] && pass "uczeń 2: zmiana hasła startowego" || fail "uczeń 2: hasło $STATUS"

request POST /api/admin/users/generate "$ADMIN_JAR" "$ADMIN_CSRF" '{"role":"kitchen","count":1}'
K_LOGIN=$(printf '%s' "$BODY" | jq -r '.accounts[0].login')
K_PASS=$(printf '%s' "$BODY" | jq -r '.accounts[0].password')
K_JAR="$TMP/kitchen.jar"
login "$K_LOGIN" "$K_PASS" "$K_JAR"
K_CSRF="$CSRF"
request POST /api/auth/change-password "$K_JAR" "$K_CSRF" \
  '{"current_password":"'"$K_PASS"'","new_password":"KuchniaHaslo12"}'
[ "$STATUS" = "200" ] && pass "kuchnia: zmiana hasła startowego" || fail "kuchnia: hasło $STATUS"

ITEM=$(db_scalar "SELECT id FROM menu_items ORDER BY id LIMIT 1;")
[ -n "$ITEM" ] && pass "menu: pozycja testowa" || fail "menu: brak pozycji"
request POST "/api/staff/menu/$ITEM/stock" "$ADMIN_JAR" "$ADMIN_CSRF" '{"set":5}'
[ "$STATUS" = "200" ] && pass "stan: ustawiono 5 szt." || fail "stan: $STATUS ($BODY)"

TODAY=$(TZ=Europe/Warsaw date +%F)
TOMORROW=$(TZ=Europe/Warsaw date -d "$TODAY +1 day" +%F)

json_order() {
  local date=$1 break_id=$2 qty=$3 key=$4 time=${5:-11:35}
  local break_json="null"
  [ "$break_id" != "0" ] && break_json=$break_id
  printf '{"items":[{"menu_item_id":%s,"quantity":%s}],"payment_method":"cash","pickup_time":"%s","pickup_date":"%s","break_id":%s,"idempotency_key":"%s"}' \
    "$ITEM" "$qty" "$time" "$date" "$break_json" "$key"
}

# ---------------------------------------------------------------- przerwy CRUD
request GET /api/admin/breaks "$ADMIN_JAR" "" ""
EMPTY=$(printf '%s' "$BODY" | jq 'length')
[ "$STATUS" = "200" ] && [ "$EMPTY" = "0" ] \
  && pass "przerwy: pusta lista na start" \
  || fail "przerwy: start=$STATUS/$EMPTY"

request POST /api/admin/breaks "$ADMIN_JAR" "$ADMIN_CSRF" \
  '{"name":"Długa przerwa","start_time":"11:30","end_time":"11:50","sort_order":1}'
BREAK_LONG=$(printf '%s' "$BODY" | jq -r '.id // empty')
[ "$STATUS" = "201" ] && [ -n "$BREAK_LONG" ] \
  && pass "przerwy: dodano długą przerwę" \
  || fail "przerwy: dodanie -> $STATUS ($BODY)"

request POST /api/admin/breaks "$ADMIN_JAR" "$ADMIN_CSRF" \
  '{"name":"Długa przerwa","start_time":"11:30","end_time":"11:50"}'
[ "$STATUS" = "409" ] && pass "przerwy: duplikat nazwy odrzucony (409)" \
  || fail "przerwy: duplikat -> $STATUS ($BODY)"

request POST /api/admin/breaks "$ADMIN_JAR" "$ADMIN_CSRF" \
  '{"name":"Zła przerwa","start_time":"11:50","end_time":"11:30"}'
[ "$STATUS" = "400" ] && pass "przerwy: odwrócone godziny odrzucone (400)" \
  || fail "przerwy: złe godziny -> $STATUS ($BODY)"

request POST /api/admin/breaks "$ADMIN_JAR" "$ADMIN_CSRF" \
  '{"name":"Krótka przerwa","start_time":"12:00","end_time":"12:10","sort_order":2}'
BREAK_SHORT=$(printf '%s' "$BODY" | jq -r '.id // empty')
[ "$STATUS" = "201" ] && pass "przerwy: dodano krótką przerwę" \
  || fail "przerwy: druga przerwa -> $STATUS ($BODY)"

request GET /api/staff/breaks "$K_JAR" "" ""
STAFF_BREAKS=$(printf '%s' "$BODY" | jq 'length')
[ "$STATUS" = "200" ] && [ "$STAFF_BREAKS" = "2" ] \
  && pass "przerwy: kuchnia widzi aktywne przerwy" \
  || fail "przerwy: staff=$STATUS/$STAFF_BREAKS"

request GET /api/student/menu "$S1_JAR" "" ""
MENU_BREAKS=$(printf '%s' "$BODY" | jq '.breaks | length')
MENU_AHEAD=$(printf '%s' "$BODY" | jq -r '.order_days_ahead')
MENU_PER_BREAK=$(printf '%s' "$BODY" | jq -r '.max_item_quantity_per_break')
[ "$MENU_BREAKS" = "2" ] && [ "$MENU_AHEAD" = "0" ] && [ "$MENU_PER_BREAK" = "0" ] \
  && pass "menu ucznia: przerwy i limity widoczne" \
  || fail "menu ucznia: breaks=$MENU_BREAKS ahead=$MENU_AHEAD per=$MENU_PER_BREAK"

# ---------------------------------------------------------------- walidacja zamówień
request POST /api/student/orders "$S1_JAR" "$S1_CSRF" "$(json_order "$TODAY" 0 1 brk-no-break)"
[ "$STATUS" = "400" ] && pass "zamówienie bez przerwy odrzucone (400)" \
  || fail "zamówienie bez przerwy -> $STATUS ($BODY)"

request POST /api/student/orders "$S1_JAR" "$S1_CSRF" "$(json_order "$TODAY" "$BREAK_LONG" 1 brk-time 12:30)"
[ "$STATUS" = "400" ] && pass "godzina poza przerwą odrzucona (400)" \
  || fail "godzina poza przerwą -> $STATUS ($BODY)"

request POST /api/student/orders "$S1_JAR" "$S1_CSRF" "$(json_order "$TOMORROW" "$BREAK_LONG" 1 brk-ahead-off)"
[ "$STATUS" = "400" ] && pass "zamówienie na jutro przy order_days_ahead=0 odrzucone (400)" \
  || fail "zamówienie na jutro -> $STATUS ($BODY)"

# Auto-oznaczanie nieodebranych wyłączone: ten test sprawdza przerwy,
# a nie upływ terminu odbioru.
request PUT /api/admin/settings "$ADMIN_JAR" "$ADMIN_CSRF" \
  '{"order_days_ahead":1,"max_item_quantity_per_break":2,"one_active_order_per_break":true,"pickup_limit_minutes":0}'
[ "$STATUS" = "200" ] && pass "ustawienia: wyprzedzenie 1 dzień i limity przerwy" \
  || fail "ustawienia: $STATUS ($BODY)"

STOCK_BEFORE=$(db_scalar "SELECT stock_remaining FROM menu_items WHERE id=$ITEM;")

request POST /api/student/orders "$S1_JAR" "$S1_CSRF" "$(json_order "$TOMORROW" "$BREAK_LONG" 2 brk-future-1)"
ORDER_FUTURE=$(printf '%s' "$BODY" | jq -r '.id // empty')
FUTURE_DATE=$(db_scalar "SELECT pickup_date FROM orders WHERE id=$ORDER_FUTURE;")
FUTURE_BREAK=$(db_scalar "SELECT break_id FROM orders WHERE id=$ORDER_FUTURE;")
FUTURE_RESERVED=$(db_scalar "SELECT COALESCE(SUM(stock_reserved),0) FROM order_items WHERE order_id=$ORDER_FUTURE;")
STOCK_AFTER_FUTURE=$(db_scalar "SELECT stock_remaining FROM menu_items WHERE id=$ITEM;")
[ "$STATUS" = "201" ] && [ "$FUTURE_DATE" = "$TOMORROW" ] && [ "$FUTURE_BREAK" = "$BREAK_LONG" ] \
  && pass "wyprzedzenie: zamówienie na jutro zapisane z przerwą" \
  || fail "wyprzedzenie: $STATUS date=$FUTURE_DATE break=$FUTURE_BREAK"
[ "$FUTURE_RESERVED" = "0" ] && [ "$STOCK_AFTER_FUTURE" = "$STOCK_BEFORE" ] \
  && pass "wyprzedzenie: przyszłe zamówienie nie rezerwuje dzisiejszego stanu" \
  || fail "wyprzedzenie: reserved=$FUTURE_RESERVED stock=$STOCK_AFTER_FUTURE/$STOCK_BEFORE"

request POST /api/student/orders "$S1_JAR" "$S1_CSRF" "$(json_order "$TOMORROW" "$BREAK_LONG" 1 brk-future-2)"
[ "$STATUS" = "409" ] && pass "limit: jedno aktywne zamówienie na przerwę (409)" \
  || fail "limit: drugie zamówienie -> $STATUS ($BODY)"

request POST /api/student/orders "$S2_JAR" "$S2_CSRF" "$(json_order "$TOMORROW" "$BREAK_LONG" 3 brk-limit-3)"
[ "$STATUS" = "409" ] && pass "limit: 3 szt. jednej pozycji na przerwę odrzucone (409)" \
  || fail "limit: 3 szt. -> $STATUS ($BODY)"

request POST /api/student/orders "$S2_JAR" "$S2_CSRF" "$(json_order "$TOMORROW" "$BREAK_LONG" 2 brk-limit-2)"
ORDER_FUTURE_2=$(printf '%s' "$BODY" | jq -r '.id // empty')
[ "$STATUS" = "201" ] && [ -n "$ORDER_FUTURE_2" ] \
  && pass "limit: 2 szt. mieszczą się w limicie przerwy" \
  || fail "limit: 2 szt. -> $STATUS ($BODY)"

request POST /api/student/orders "$S2_JAR" "$S2_CSRF" "$(json_order "$TODAY" "$BREAK_LONG" 2 brk-today-1)"
ORDER_TODAY=$(printf '%s' "$BODY" | jq -r '.id // empty')
STOCK_TODAY=$(db_scalar "SELECT stock_remaining FROM menu_items WHERE id=$ITEM;")
[ "$STATUS" = "201" ] && [ "$STOCK_TODAY" = "$((STOCK_BEFORE - 2))" ] \
  && pass "stan: dzisiejsze zamówienie rezerwuje 2 szt." \
  || fail "stan: dzisiaj -> $STATUS stock=$STOCK_TODAY (oczekiwano $((STOCK_BEFORE - 2)))"

# ---------------------------------------------------------------- dzień odbioru
request GET "/api/student/orders/$ORDER_FUTURE/qr.svg" "$S1_JAR" "" ""
[ "$STATUS" = "409" ] && pass "QR: przyszłe zamówienie nie ma aktywnego kodu" \
  || fail "QR: przyszłe -> $STATUS"

request PATCH "/api/staff/orders/$ORDER_FUTURE" "$K_JAR" "$K_CSRF" '{"status":"issued"}'
[ "$STATUS" = "403" ] && pass "kuchnia: nie wydaje zamówienia na przyszły dzień (403)" \
  || fail "kuchnia: przyszłe wydanie -> $STATUS ($BODY)"

# Anulowanie przyszłego zamówienia nie rusza dzisiejszego stanu.
request POST "/api/student/orders/$ORDER_FUTURE_2/cancel" "$S2_JAR" "$S2_CSRF" '{}'
STOCK_AFTER_CANCEL_FUTURE=$(db_scalar "SELECT stock_remaining FROM menu_items WHERE id=$ITEM;")
[ "$STATUS" = "200" ] && [ "$STOCK_AFTER_CANCEL_FUTURE" = "$STOCK_TODAY" ] \
  && pass "anulowanie: przyszłe zamówienie nie zwraca dzisiejszego stanu" \
  || fail "anulowanie: przyszłe -> $STATUS stock=$STOCK_AFTER_CANCEL_FUTURE/$STOCK_TODAY"

request POST "/api/student/orders/$ORDER_TODAY/cancel" "$S2_JAR" "$S2_CSRF" '{}'
STOCK_AFTER_CANCEL_TODAY=$(db_scalar "SELECT stock_remaining FROM menu_items WHERE id=$ITEM;")
[ "$STATUS" = "200" ] && [ "$STOCK_AFTER_CANCEL_TODAY" = "$STOCK_BEFORE" ] \
  && pass "anulowanie: dzisiejsze zamówienie zwraca stan" \
  || fail "anulowanie: dzisiaj -> $STATUS stock=$STOCK_AFTER_CANCEL_TODAY/$STOCK_BEFORE"

# ---------------------------------------------------------------- raport
request GET "/api/staff/report?date=$TOMORROW" "$K_JAR" "" ""
REPORT_ORDERS=$(printf '%s' "$BODY" | jq -r '.orders // empty')
LONG_ORDERS=$(printf '%s' "$BODY" | jq -r --arg id "$BREAK_LONG" '.breaks[] | select(.break_id == ($id|tonumber)) | .orders' | head -n1)
[ "$REPORT_ORDERS" = "2" ] && [ "$LONG_ORDERS" = "2" ] \
  && pass "raport: jutro pokazuje 2 zamówienia i rozbicie na przerwę" \
  || fail "raport: jutro orders=$REPORT_ORDERS long=$LONG_ORDERS"

curl -s --noproxy '*' -o "$TMP/report.csv" -H "Host: $HOSTADDR" -H "Origin: $ORIGIN" \
  -b "$K_JAR" "http://$HOSTADDR/api/staff/report.csv?date=$TOMORROW"
grep -q '^przerwa;' "$TMP/report.csv" && grep -q 'Długa przerwa' "$TMP/report.csv" \
  && pass "CSV: zawiera rozbicie na przerwy" \
  || fail "CSV: brak rozbicia na przerwy"

# ---------------------------------------------------------------- usuwanie i wyłączanie
request DELETE "/api/admin/breaks/$BREAK_LONG" "$ADMIN_JAR" "$ADMIN_CSRF" ""
[ "$STATUS" = "409" ] && pass "przerwy: nie można usunąć przerwy z zamówieniami (409)" \
  || fail "przerwy: usunięcie używanej -> $STATUS ($BODY)"

request POST /api/admin/breaks "$ADMIN_JAR" "$ADMIN_CSRF" \
  '{"name":"Testowa do usunięcia","start_time":"13:00","end_time":"13:10"}'
BREAK_TMP=$(printf '%s' "$BODY" | jq -r '.id // empty')
request DELETE "/api/admin/breaks/$BREAK_TMP" "$ADMIN_JAR" "$ADMIN_CSRF" ""
[ "$STATUS" = "204" ] && pass "przerwy: nieużywana przerwa usunięta (204)" \
  || fail "przerwy: usunięcie nieużywanej -> $STATUS ($BODY)"

request PUT "/api/admin/breaks/$BREAK_LONG" "$ADMIN_JAR" "$ADMIN_CSRF" \
  '{"name":"Długa przerwa","start_time":"11:30","end_time":"11:50","sort_order":1,"active":false}'
[ "$STATUS" = "200" ] && pass "przerwy: wyłączenie zamiast usuwania" \
  || fail "przerwy: wyłączenie -> $STATUS ($BODY)"

request PUT "/api/admin/breaks/$BREAK_SHORT" "$ADMIN_JAR" "$ADMIN_CSRF" \
  '{"name":"Krótka przerwa","start_time":"12:00","end_time":"12:10","sort_order":2,"active":false}'
request GET /api/student/menu "$S1_JAR" "" ""
ACTIVE_BREAKS=$(printf '%s' "$BODY" | jq '.breaks | length')
[ "$ACTIVE_BREAKS" = "0" ] && pass "przerwy: wyłączone znikają z formularza ucznia" \
  || fail "przerwy: aktywne=$ACTIVE_BREAKS"

# Bez aktywnych przerw zamówienie nie wymaga przerwy; limity per przerwa
# nie mają wtedy zastosowania.
request POST /api/student/orders "$S2_JAR" "$S2_CSRF" "$(json_order "$TODAY" 0 1 brk-no-break-after)"
[ "$STATUS" = "201" ] && pass "zamówienie bez przerw działa po ich wyłączeniu" \
  || fail "zamówienie bez przerw po wyłączeniu -> $STATUS ($BODY)"

request PUT /api/admin/settings "$ADMIN_JAR" "$ADMIN_CSRF" \
  '{"order_days_ahead":0,"max_item_quantity_per_break":0,"one_active_order_per_break":false,"pickup_limit_minutes":120}'
[ "$STATUS" = "200" ] && pass "ustawienia: przywrócone wartości domyślne" \
  || fail "ustawienia: reset -> $STATUS ($BODY)"

printf '\nWynik: %d błędów\n' "$FAILS"
exit "$FAILS"
