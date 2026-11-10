#!/usr/bin/env bash
# UWAGA: skrypt testowy. Używa wyłącznie danych testowych i izolowanej bazy
# w /tmp/opencode. Nie uruchamiaj go na prawdziwej instancji.
#
# Sprawdza sekcję 3: idempotencję, transakcyjne wyścigi, płatności lokalne,
# ograniczenia kuchni, nieodebrane zamówienia oraz raport/CSV.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN="$ROOT/target/debug/zsl_order"
BASE="/tmp/opencode/zsl-section3"
TMP="$BASE/tmp"
DB="$BASE/baza.db"
HOSTADDR="127.0.0.1:38192"
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

ASYNC_PIDS=()

async_request() {
  local tag=$1 method=$2 path=$3 jar=$4 csrf=$5 data=$6
  curl -s --noproxy '*' -X "$method" -H "Host: $HOSTADDR" -H "Origin: $ORIGIN" \
    -b "$jar" -H "X-CSRF-Token: $csrf" -H 'Content-Type: application/json' \
    --data-binary "$data" -o "$TMP/$tag.body" -w '%{http_code}\n' >"$TMP/$tag.status" \
    "http://$HOSTADDR$path" &
  ASYNC_PIDS+=("$!")
}

wait_async() {
  if [ "${#ASYNC_PIDS[@]}" -gt 0 ]; then
    wait "${ASYNC_PIDS[@]}" 2>/dev/null || true
    ASYNC_PIDS=()
  fi
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

json_order() {
  local method=$1 key=$2
  printf '{"items":[{"menu_item_id":%s,"quantity":1}],"payment_method":"%s","pickup_time":"12:30","idempotency_key":"%s"}' \
    "$ITEM" "$method" "$key"
}

create_order() {
  request POST /api/student/orders "$S1_JAR" "$S1_CSRF" "$(json_order "$1" "$2")"
}

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
  "login_lockout_minutes": 15,
  "pickup_max_failures": 8,
  "pickup_ip_max_failures": 100,
  "pickup_window_minutes": 10,
  "pickup_lockout_minutes": 15
}
JSON
cat >"$BASE/.env" <<'ENV'
DEPLOY_MODE=local
BIND_ADDR=127.0.0.1:38192
COOKIE_SECURE=false
ENV

start_server || { echo "Serwer nie wystartował"; exit 1; }

for _ in $(seq 1 100); do
  [ -f "$BASE/admin_start.txt" ] && break
  sleep 0.1
done
ADMIN_LOGIN=$(sed -n 's/^Login:  //p' "$BASE/admin_start.txt" | head -n1)
ADMIN_PASS=$(sed -n 's/^Hasło:  //p' "$BASE/admin_start.txt" | head -n1)

# ---------------------------------------------------------------- logowanie
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
[ -n "$S1_LOGIN" ] && pass "import: konto ucznia" || fail "import: brak konta ($STATUS/$BODY)"

S1_JAR="$TMP/s1.jar"
login "$S1_LOGIN" "$S1_PASS" "$S1_JAR"
S1_CSRF="$CSRF"
request POST /api/auth/change-password "$S1_JAR" "$S1_CSRF" \
  '{"current_password":"'"$S1_PASS"'","new_password":"UczenHaslo10"}'
[ "$STATUS" = "200" ] && pass "uczeń: zmiana hasła startowego" || fail "uczeń: zmiana hasła $STATUS"

request POST /api/admin/users/generate "$ADMIN_JAR" "$ADMIN_CSRF" '{"role":"kitchen","count":1}'
K_LOGIN=$(printf '%s' "$BODY" | jq -r '.accounts[0].login')
K_PASS=$(printf '%s' "$BODY" | jq -r '.accounts[0].password')
K_JAR="$TMP/kitchen.jar"
login "$K_LOGIN" "$K_PASS" "$K_JAR"
K_CSRF="$CSRF"
request POST /api/auth/change-password "$K_JAR" "$K_CSRF" \
  '{"current_password":"'"$K_PASS"'","new_password":"KuchniaHaslo12"}'
[ "$STATUS" = "200" ] && pass "kuchnia: zmiana hasła startowego" || fail "kuchnia: zmiana hasła $STATUS"

request GET /api/student/menu "$S1_JAR" "" ""
ITEM=$(printf '%s' "$BODY" | jq -r '.items[0].id // empty')
ITEM_PRICE=$(printf '%s' "$BODY" | jq -r '.items[0].price_cents // empty')
[ -n "$ITEM" ] && [ -n "$ITEM_PRICE" ] && pass "menu: pozycja testowa" || fail "menu: brak pozycji"

# ---------------------------------------------------------------- idempotencja
request POST /api/student/orders "$S1_JAR" "$S1_CSRF" \
  '{"items":[{"menu_item_id":'"$ITEM"',"quantity":1}],"payment_method":"cash"}'
[ "$STATUS" = "400" ] && pass "idempotencja: brak klucza odrzucony (400)" || fail "idempotencja: brak klucza -> $STATUS"

request POST /api/student/orders "$S1_JAR" "$S1_CSRF" \
  '{"items":[{"menu_item_id":'"$ITEM"',"quantity":1,"price_cents":1}],"payment_method":"cash","pickup_time":"12:30","idempotency_key":"smoke-idem-0001","status":"completed","payment_status":"paid","total_cents":1}'
ORDER1=$(printf '%s' "$BODY" | jq -r '.id // empty')
TOTAL1=$(printf '%s' "$BODY" | jq -r '.total_cents // empty')
STATUS1=$(printf '%s' "$BODY" | jq -r '.status // empty')
PAY1=$(printf '%s' "$BODY" | jq -r '.payment_status // empty')
[ "$STATUS" = "201" ] && pass "zamówienie: utworzone (201)" || fail "zamówienie: create $STATUS ($BODY)"
[ "$TOTAL1" = "$ITEM_PRICE" ] && pass "cena: policzona z bazy, nie z żądania" || fail "cena: total=$TOTAL1 oczekiwano $ITEM_PRICE"
[ "$STATUS1" = "new" ] && [ "$PAY1" = "unpaid" ] \
  && pass "uczeń: nie ustawia statusu ani płatności" \
  || fail "uczeń: status=$STATUS1 płatność=$PAY1"

request POST /api/student/orders "$S1_JAR" "$S1_CSRF" \
  '{"items":[{"menu_item_id":'"$ITEM"',"quantity":1,"price_cents":1}],"payment_method":"cash","pickup_time":"12:30","idempotency_key":"smoke-idem-0001","status":"completed","payment_status":"paid","total_cents":1}'
ORDER1_DUP=$(printf '%s' "$BODY" | jq -r '.id // empty')
[ "$STATUS" = "200" ] && [ "$ORDER1_DUP" = "$ORDER1" ] \
  && pass "idempotencja: powtórzenie zwraca to samo zamówienie (200)" \
  || fail "idempotencja: duplikat -> $STATUS id=$ORDER1_DUP"
COUNT1=$(db_scalar "SELECT COUNT(*) FROM orders WHERE user_id=(SELECT id FROM users WHERE login='$S1_LOGIN');")
[ "$COUNT1" = "1" ] && pass "idempotencja: w bazie jedno zamówienie" || fail "idempotencja: zamówień=$COUNT1"

# ---------------------------------------------------------------- wyścig anulowań
create_order cash smoke-idem-0002
ORDER2=$(printf '%s' "$BODY" | jq -r '.id // empty')
[ "$STATUS" = "201" ] && pass "wyścig: zamówienie do anulowania" || fail "wyścig: create2 $STATUS"

async_request cancel-a POST "/api/student/orders/$ORDER2/cancel" "$S1_JAR" "$S1_CSRF" '{}'
async_request cancel-b POST "/api/student/orders/$ORDER2/cancel" "$S1_JAR" "$S1_CSRF" '{}'
wait_async
OK_CANCEL=$(cat "$TMP/cancel-a.status" "$TMP/cancel-b.status" | grep -c '^200$')
CONFLICT_CANCEL=$(cat "$TMP/cancel-a.status" "$TMP/cancel-b.status" | grep -c '^409$')
[ "$OK_CANCEL" = "1" ] && [ "$CONFLICT_CANCEL" = "1" ] \
  && pass "wyścig: dwa anulowania -> jedno 200, jedno 409" \
  || fail "wyścig: anulowania 200=$OK_CANCEL 409=$CONFLICT_CANCEL"
ORDER2_STATUS=$(db_scalar "SELECT status FROM orders WHERE id=$ORDER2;")
[ "$ORDER2_STATUS" = "cancelled" ] && pass "wyścig: spójny stan po anulowaniu" || fail "wyścig: status=$ORDER2_STATUS"

# ---------------------------------------------------------------- wyścig statusów + płatność
create_order card smoke-idem-0003
ORDER3=$(printf '%s' "$BODY" | jq -r '.id // empty')
[ "$STATUS" = "201" ] && pass "płatność: zamówienie kartą" || fail "płatność: create3 $STATUS"

async_request status-a PATCH "/api/staff/orders/$ORDER3" "$K_JAR" "$K_CSRF" '{"status":"preparing"}'
async_request status-b PATCH "/api/staff/orders/$ORDER3" "$K_JAR" "$K_CSRF" '{"status":"preparing"}'
wait_async
BAD_STATUS=$(cat "$TMP/status-a.status" "$TMP/status-b.status" | grep -cvE '^(200|409)$')
AT_LEAST_ONE=$(cat "$TMP/status-a.status" "$TMP/status-b.status" | grep -c '^200$')
[ "$BAD_STATUS" = "0" ] && [ "$AT_LEAST_ONE" -ge 1 ] \
  && pass "wyścig: równoległe zmiany statusu bez błędu 5xx" \
  || fail "wyścig: statusy $(cat "$TMP/status-a.status" "$TMP/status-b.status" | tr '\n' ' ')"

request PATCH "/api/staff/orders/$ORDER3" "$K_JAR" "$K_CSRF" '{"status":"ready"}'
[ "$STATUS" = "200" ] && pass "kuchnia: gotowe do odbioru" || fail "kuchnia: ready $STATUS ($BODY)"
request PATCH "/api/staff/orders/$ORDER3" "$K_JAR" "$K_CSRF" '{"status":"completed"}'
PAID_BY=$(printf '%s' "$BODY" | jq -r '.paid_by // empty')
PAID_AT=$(printf '%s' "$BODY" | jq -r '.paid_at // empty')
PAY_STATUS=$(printf '%s' "$BODY" | jq -r '.payment_status // empty')
[ "$STATUS" = "200" ] && [ "$PAY_STATUS" = "paid" ] && [ "$PAID_BY" = "$K_LOGIN" ] && [ -n "$PAID_AT" ] \
  && pass "płatność: wydanie zapisuje paid_at i paid_by" \
  || fail "płatność: $STATUS paid=$PAY_STATUS by=$PAID_BY at=$PAID_AT"
DB_PAID_BY=$(db_scalar "SELECT paid_by FROM orders WHERE id=$ORDER3;")
DB_K_ID=$(db_scalar "SELECT id FROM users WHERE login='$K_LOGIN';")
[ "$DB_PAID_BY" = "$DB_K_ID" ] && pass "płatność: paid_by wskazuje konto kuchni" || fail "płatność: paid_by=$DB_PAID_BY"

# ---------------------------------------------------------------- inny dzień
S1_ID=$(db_scalar "SELECT id FROM users WHERE login='$S1_LOGIN';")
YDAY=$(date -d 'yesterday 12:00' +%s)
OLD_ID=$(db_scalar "INSERT INTO orders (user_id,code,qr_token,status,payment_method,payment_status,total_cents,pickup_time,created_at,updated_at) VALUES ($S1_ID,'OLD999','oldtoken999','new','cash','unpaid',$ITEM_PRICE,'12:00',$YDAY,$YDAY); SELECT last_insert_rowid();")
request PATCH "/api/staff/orders/$OLD_ID" "$K_JAR" "$K_CSRF" '{"status":"preparing"}'
OLD_STATUS=$(db_scalar "SELECT status FROM orders WHERE id=$OLD_ID;")
[ "$STATUS" = "403" ] && [ "$OLD_STATUS" = "new" ] \
  && pass "kuchnia: nie zmienia zamówień z innych dni (403)" \
  || fail "kuchnia: inny dzień -> $STATUS/$OLD_STATUS"

# ---------------------------------------------------------------- termin anulowania
request PUT /api/admin/settings "$ADMIN_JAR" "$ADMIN_CSRF" '{"cancel_window_minutes":1}'
[ "$STATUS" = "200" ] && pass "ustawienia: termin anulowania zapisany" || fail "ustawienia: cancel_window $STATUS"

create_order cash smoke-idem-0004
ORDER4=$(printf '%s' "$BODY" | jq -r '.id // empty')
db_scalar "UPDATE orders SET created_at = created_at - 120 WHERE id=$ORDER4;" >/dev/null
request POST "/api/student/orders/$ORDER4/cancel" "$S1_JAR" "$S1_CSRF" '{}'
[ "$STATUS" = "409" ] && pass "anulowanie: po terminie odrzucone (409)" || fail "anulowanie: po terminie -> $STATUS ($BODY)"

request PUT /api/admin/settings "$ADMIN_JAR" "$ADMIN_CSRF" '{"cancel_window_minutes":0}'
request POST "/api/student/orders/$ORDER4/cancel" "$S1_JAR" "$S1_CSRF" '{}'
[ "$STATUS" = "200" ] && pass "anulowanie: 0 = bez limitu, dozwolone" || fail "anulowanie: bez limitu -> $STATUS ($BODY)"

# ---------------------------------------------------------------- nieodebrane
request PUT /api/admin/settings "$ADMIN_JAR" "$ADMIN_CSRF" \
  '{"pickup_limit_minutes":1,"max_uncollected":1,"cancel_window_minutes":30}'
[ "$STATUS" = "200" ] && pass "ustawienia: limit odbioru i blokada zapisane" || fail "ustawienia: uncollected $STATUS"

create_order cash smoke-idem-0005
ORDER5=$(printf '%s' "$BODY" | jq -r '.id // empty')
request PATCH "/api/staff/orders/$ORDER5" "$K_JAR" "$K_CSRF" '{"status":"preparing"}'
request PATCH "/api/staff/orders/$ORDER5" "$K_JAR" "$K_CSRF" '{"status":"ready"}'
[ "$STATUS" = "200" ] && pass "nieodebrane: zamówienie gotowe" || fail "nieodebrane: ready $STATUS"
db_scalar "UPDATE orders SET ready_at = unixepoch() - 120 WHERE id=$ORDER5;" >/dev/null

stop_server
start_server || { echo "Restart nieudany"; exit 1; }
for _ in $(seq 1 50); do
  [ -n "$(db_scalar "SELECT uncollected_at FROM orders WHERE id=$ORDER5;")" ] && break
  sleep 0.2
done
UNCOLLECTED=$(db_scalar "SELECT uncollected_at FROM orders WHERE id=$ORDER5;")
[ -n "$UNCOLLECTED" ] && pass "nieodebrane: automatycznie oznaczone po restarcie" || fail "nieodebrane: brak znacznika"

request GET /api/student/orders "$S1_JAR" "" ""
EFFECTIVE=$(printf '%s' "$BODY" | jq -r --argjson id "$ORDER5" '.[] | select(.id == $id) | .status')
[ "$EFFECTIVE" = "uncollected" ] && pass "nieodebrane: API pokazuje status uncollected" || fail "nieodebrane: status=$EFFECTIVE"

create_order cash smoke-idem-0006
[ "$STATUS" = "409" ] && pass "nieodebrane: blokada nowych zamówień po limicie (409)" || fail "nieodebrane: blokada -> $STATUS ($BODY)"

request PATCH "/api/staff/orders/$ORDER5" "$K_JAR" "$K_CSRF" '{"status":"preparing"}'
[ "$STATUS" = "409" ] && pass "nieodebrane: kuchnia nie zmienia zamkniętego (409)" || fail "nieodebrane: kuchnia -> $STATUS"

# ---------------------------------------------------------------- raport i CSV
REPORT_DATE=$(date +%F)
DAY_START=$(date -d "$REPORT_DATE" +%s)
DAY_END=$(date -d "$REPORT_DATE +1 day" +%s)
request GET "/api/staff/report?date=$REPORT_DATE" "$K_JAR" "" ""
REPORT_PAID=$(printf '%s' "$BODY" | jq -r '.paid_cents // empty')
REPORT_ORDERS=$(printf '%s' "$BODY" | jq -r '.orders // empty')
REPORT_UNCOLLECTED=$(printf '%s' "$BODY" | jq -r '.uncollected_orders // empty')
REPORT_CASH=$(printf '%s' "$BODY" | jq -r '.cash.paid_cents // empty')
REPORT_CARD=$(printf '%s' "$BODY" | jq -r '.card.paid_cents // empty')

DB_ORDERS=$(db_scalar "SELECT COUNT(*) FROM orders WHERE created_at >= $DAY_START AND created_at < $DAY_END;")
DB_PAID=$(db_scalar "SELECT COALESCE(SUM(total_cents),0) FROM orders WHERE created_at >= $DAY_START AND created_at < $DAY_END AND payment_status='paid' AND NOT (status='cancelled' AND uncollected_at IS NULL);")
DB_CASH=$(db_scalar "SELECT COALESCE(SUM(total_cents),0) FROM orders WHERE created_at >= $DAY_START AND created_at < $DAY_END AND payment_method='cash' AND payment_status='paid' AND NOT (status='cancelled' AND uncollected_at IS NULL);")
DB_CARD=$(db_scalar "SELECT COALESCE(SUM(total_cents),0) FROM orders WHERE created_at >= $DAY_START AND created_at < $DAY_END AND payment_method='card' AND payment_status='paid' AND NOT (status='cancelled' AND uncollected_at IS NULL);")
DB_UNCOLLECTED=$(db_scalar "SELECT COUNT(*) FROM orders WHERE created_at >= $DAY_START AND created_at < $DAY_END AND uncollected_at IS NOT NULL;")

[ "$REPORT_ORDERS" = "$DB_ORDERS" ] && pass "raport: liczba zamówień zgodna z bazą" || fail "raport: orders=$REPORT_ORDERS db=$DB_ORDERS"
[ "$REPORT_PAID" = "$DB_PAID" ] && pass "raport: suma opłaconych zgodna z bazą" || fail "raport: paid=$REPORT_PAID db=$DB_PAID"
[ "$REPORT_CASH" = "$DB_CASH" ] && pass "raport: gotówka zgodna z bazą" || fail "raport: cash=$REPORT_CASH db=$DB_CASH"
[ "$REPORT_CARD" = "$DB_CARD" ] && pass "raport: karta zgodna z bazą" || fail "raport: card=$REPORT_CARD db=$DB_CARD"
[ "$REPORT_UNCOLLECTED" = "$DB_UNCOLLECTED" ] && pass "raport: nieodebrane zgodne z bazą" || fail "raport: uncollected=$REPORT_UNCOLLECTED db=$DB_UNCOLLECTED"

CSV_HEADERS=$(curl -s --noproxy '*' -D - -o "$TMP/report.csv" -H "Host: $HOSTADDR" \
  -H "Origin: $ORIGIN" -b "$K_JAR" "http://$HOSTADDR/api/staff/report.csv?date=$REPORT_DATE" \
  | tr -d '\r')
CSV_CACHE=$(printf '%s' "$CSV_HEADERS" | sed -n 's/^[Cc]ache-[Cc]ontrol: //p')
[ "$CSV_CACHE" = "no-store" ] && pass "CSV: Cache-Control no-store" || fail "CSV: cache=$CSV_CACHE"
grep -q '"gotówka"' "$TMP/report.csv" && grep -q '"karta"' "$TMP/report.csv" && grep -q '"razem"' "$TMP/report.csv" \
  && pass "CSV: podział gotówka/karta/razem" \
  || fail "CSV: brak podziału metod"

request GET "/api/staff/report?date=$REPORT_DATE" "$S1_JAR" "" ""
[ "$STATUS" = "403" ] && pass "raport: uczeń nie ma dostępu (403)" || fail "raport: uczeń -> $STATUS"
request GET "/api/staff/report?date=2026-13-40" "$K_JAR" "" ""
[ "$STATUS" = "400" ] && pass "raport: nieprawidłowa data odrzucona (400)" || fail "raport: zła data -> $STATUS"

# ---------------------------------------------------------------- korekta i odblokowanie
request PATCH "/api/staff/orders/$ORDER5" "$ADMIN_JAR" "$ADMIN_CSRF" '{"status":"completed"}'
CORRECTED=$(db_scalar "SELECT COALESCE(uncollected_at,'') || '|' || status FROM orders WHERE id=$ORDER5;")
[ "$STATUS" = "200" ] && [ "$CORRECTED" = "|completed" ] \
  && pass "korekta: admin zdejmuje znacznik nieodebranego" \
  || fail "korekta: $STATUS/$CORRECTED"

create_order cash smoke-idem-0007
[ "$STATUS" = "201" ] && pass "blokada: po korekcie zamówienie znów możliwe" || fail "blokada: po korekcie -> $STATUS ($BODY)"

request GET "/api/staff/report?date=$REPORT_DATE" "$K_JAR" "" ""
REPORT_PAID2=$(printf '%s' "$BODY" | jq -r '.paid_cents // empty')
DB_PAID2=$(db_scalar "SELECT COALESCE(SUM(total_cents),0) FROM orders WHERE created_at >= $DAY_START AND created_at < $DAY_END AND payment_status='paid' AND NOT (status='cancelled' AND uncollected_at IS NULL);")
[ "$REPORT_PAID2" = "$DB_PAID2" ] && pass "raport: sumy nadal zgodne po korekcie" || fail "raport: po korekcie $REPORT_PAID2 != $DB_PAID2"

printf '\nWynik: %d błędów\n' "$FAILS"
exit "$FAILS"
