#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
# UWAGA: skrypt testowy. Używa wyłącznie danych testowych i izolowanej bazy
# w /tmp/opencode. Nie uruchamiaj go na prawdziwej instancji.
#
# Sprawdza sekcję 3: idempotencję, transakcyjne wyścigi, płatności lokalne,
# ograniczenia kuchni, nieodebrane zamówienia oraz raport/CSV.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN="$ROOT/target/debug/zsl-order"
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

# Drugi uczeń do testu idempotencji per konto i limitu otwartych zamówień.
request POST /api/admin/users/generate "$ADMIN_JAR" "$ADMIN_CSRF" '{"role":"student","count":1}'
S2_LOGIN=$(printf '%s' "$BODY" | jq -r '.accounts[0].login')
S2_PASS=$(printf '%s' "$BODY" | jq -r '.accounts[0].password')
[ -n "$S2_LOGIN" ] && pass "import: drugie konto ucznia" || fail "import: brak drugiego konta ($STATUS/$BODY)"
S2_JAR="$TMP/s2.jar"
login "$S2_LOGIN" "$S2_PASS" "$S2_JAR"
S2_CSRF="$CSRF"
request POST /api/auth/change-password "$S2_JAR" "$S2_CSRF" \
  '{"current_password":"'"$S2_PASS"'","new_password":"UczenHaslo11"}'
[ "$STATUS" = "200" ] && pass "uczeń 2: zmiana hasła startowego" || fail "uczeń 2: zmiana hasła $STATUS"

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

# Ten sam klucz innego ucznia nie może ujawnić ani zwrócić cudzego zamówienia.
request POST /api/student/orders "$S2_JAR" "$S2_CSRF" "$(json_order cash smoke-idem-0001)"
CROSS_ID=$(printf '%s' "$BODY" | jq -r '.id // empty')
S2_ID=$(db_scalar "SELECT id FROM users WHERE login='$S2_LOGIN';")
[ "$STATUS" = "201" ] && [ -n "$CROSS_ID" ] && [ "$CROSS_ID" != "$ORDER1" ] \
  && pass "idempotencja: cudzy klucz tworzy własne zamówienie (201)" \
  || fail "idempotencja: cudzy klucz -> $STATUS id=$CROSS_ID (S1=$ORDER1)"
request GET /api/student/orders "$S2_JAR" "" ""
LEAK=$(printf '%s' "$BODY" | jq --argjson id "$ORDER1" '[.[] | select(.id == $id)] | length')
S2_ORDERS=$(printf '%s' "$BODY" | jq 'length')
[ "$LEAK" = "0" ] && [ "$S2_ORDERS" = "1" ] \
  && pass "idempotencja: uczeń widzi tylko własne zamówienie" \
  || fail "idempotencja: wyciek=$LEAK liczba=$S2_ORDERS"

# Limit aktywnych zamówień jest konfigurowalny (max_open_orders).
request PUT /api/admin/settings "$ADMIN_JAR" "$ADMIN_CSRF" '{"max_open_orders":1}'
request POST /api/student/orders "$S2_JAR" "$S2_CSRF" "$(json_order cash smoke-idem-s2-0002)"
[ "$STATUS" = "409" ] && pass "limit: max_open_orders=1 blokuje drugie zamówienie" \
  || fail "limit: max_open_orders=1 -> $STATUS ($BODY)"
request PUT /api/admin/settings "$ADMIN_JAR" "$ADMIN_CSRF" '{"max_open_orders":3}'

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

request PATCH "/api/staff/orders/$ORDER3" "$K_JAR" "$K_CSRF" '{"payment_status":"unpaid"}'
[ "$STATUS" = "403" ] && pass "płatność: kuchnia nie cofa opłacenia (403)" \
  || fail "płatność: kuchnia cofa -> $STATUS ($BODY)"

request PATCH "/api/staff/orders/$ORDER3" "$ADMIN_JAR" "$ADMIN_CSRF" '{"payment_status":"unpaid"}'
REVERTED_PAID_AT=$(printf '%s' "$BODY" | jq -r '.paid_at // empty')
[ "$STATUS" = "200" ] && [ -z "$REVERTED_PAID_AT" ] \
  && pass "płatność: admin cofa opłacenie" \
  || fail "płatność: admin cofa -> $STATUS paid_at=$REVERTED_PAID_AT"
AUDIT_REVERT=$(db_scalar "SELECT COUNT(*) FROM audit_log WHERE action='order_payment_reverted' AND details LIKE '%id=$ORDER3,%';")
[ "$AUDIT_REVERT" -ge 1 ] && pass "płatność: cofnięcie zapisane w audycie" \
  || fail "płatność: brak wpisu audytu ($AUDIT_REVERT)"
request PATCH "/api/staff/orders/$ORDER3" "$ADMIN_JAR" "$ADMIN_CSRF" '{"payment_status":"paid"}'
[ "$STATUS" = "200" ] && pass "płatność: admin przywraca opłacenie" \
  || fail "płatność: przywrócenie -> $STATUS ($BODY)"

# ---------------------------------------------------------------- odbiór QR
create_order cash smoke-idem-qr-0001
QR_ORDER=$(printf '%s' "$BODY" | jq -r '.id // empty')
[ "$STATUS" = "201" ] && pass "QR: zamówienie testowe" || fail "QR: create $STATUS ($BODY)"
request PATCH "/api/staff/orders/$QR_ORDER" "$K_JAR" "$K_CSRF" '{"status":"preparing"}'
request PATCH "/api/staff/orders/$QR_ORDER" "$K_JAR" "$K_CSRF" '{"status":"ready"}'
[ "$STATUS" = "200" ] && pass "QR: zamówienie gotowe do wydania" || fail "QR: ready $STATUS"

QR_TOKEN=$(db_scalar "SELECT qr_token FROM orders WHERE id=$QR_ORDER;")
printf '%s' "$QR_TOKEN" | grep -Eq '^[0-9a-f]{48}$' \
  && pass "QR: token to 48 hex (bez danych osobowych)" \
  || fail "QR: nieoczekiwany token '$QR_TOKEN'"

request GET "/api/student/orders/$QR_ORDER/qr.svg" "$S1_JAR" "" ""
[ "$STATUS" = "200" ] && pass "QR: uczeń pobiera własny kod" || fail "QR: qr.svg $STATUS"

request POST /api/staff/pickup "$K_JAR" "$K_CSRF" "{\"code\":\"$QR_TOKEN\"}"
PICKED_ID=$(printf '%s' "$BODY" | jq -r '.order.id // empty')
PICKED_TOTAL=$(printf '%s' "$BODY" | jq -r '.order.total_cents // empty')
PICKED_PAYMENT=$(printf '%s' "$BODY" | jq -r '.order.payment_method // empty')
PICKED_STATUS=$(printf '%s' "$BODY" | jq -r '.order.payment_status // empty')
[ "$STATUS" = "200" ] && [ "$PICKED_ID" = "$QR_ORDER" ] \
  && [ "$PICKED_TOTAL" = "$ITEM_PRICE" ] && [ "$PICKED_PAYMENT" = "cash" ] \
  && [ "$PICKED_STATUS" = "unpaid" ] \
  && pass "QR: skan zwraca kwotę, płatność i status" \
  || fail "QR: skan $STATUS id=$PICKED_ID total=$PICKED_TOTAL payment=$PICKED_PAYMENT/$PICKED_STATUS"

# Endpoint skanowania wymaga konta kuchni lub admina.
request POST /api/staff/pickup "$S1_JAR" "$S1_CSRF" "{\"code\":\"$QR_TOKEN\"}"
[ "$STATUS" = "403" ] && pass "QR: uczeń nie może skanować (403)" \
  || fail "QR: uczeń skanuje -> $STATUS ($BODY)"

request PATCH "/api/staff/orders/$QR_ORDER" "$K_JAR" "$K_CSRF" '{"status":"completed"}'
[ "$STATUS" = "200" ] && pass "QR: wydanie zamyka zamówienie" || fail "QR: wydanie $STATUS ($BODY)"
request POST /api/staff/pickup "$K_JAR" "$K_CSRF" "{\"code\":\"$QR_TOKEN\"}"
[ "$STATUS" = "409" ] && pass "QR: kod już wydany przestaje działać (409)" \
  || fail "QR: ponowny skan wydanego -> $STATUS ($BODY)"

request POST /api/staff/pickup "$K_JAR" "$K_CSRF" '{"code":"NIE000"}'
[ "$STATUS" = "404" ] && pass "QR: nieistniejący kod (404)" \
  || fail "QR: nieistniejący kod -> $STATUS ($BODY)"

# ---------------------------------------------------------------- inny dzień
S1_ID=$(db_scalar "SELECT id FROM users WHERE login='$S1_LOGIN';")
YDAY=$(TZ=Europe/Warsaw date -d 'yesterday 12:00' +%s)
OLD_ID=$(db_scalar "INSERT INTO orders (user_id,code,qr_token,status,payment_method,payment_status,total_cents,pickup_time,created_at,updated_at) VALUES ($S1_ID,'OLD999','oldtoken999','ready','cash','unpaid',$ITEM_PRICE,'12:00',$YDAY,$YDAY); SELECT last_insert_rowid();")
request PATCH "/api/staff/orders/$OLD_ID" "$K_JAR" "$K_CSRF" '{"status":"preparing"}'
OLD_STATUS=$(db_scalar "SELECT status FROM orders WHERE id=$OLD_ID;")
[ "$STATUS" = "403" ] && [ "$OLD_STATUS" = "ready" ] \
  && pass "kuchnia: nie zmienia zamówień z innych dni (403)" \
  || fail "kuchnia: inny dzień -> $STATUS/$OLD_STATUS"
request POST /api/staff/pickup "$K_JAR" "$K_CSRF" '{"code":"oldtoken999"}'
[ "$STATUS" = "409" ] && pass "QR: kod z innego dnia odrzucony (409)" \
  || fail "QR: kod z innego dnia -> $STATUS ($BODY)"
db_scalar "DELETE FROM orders WHERE id=$OLD_ID;" >/dev/null

# Limit prób: seria nieistniejących kodów musi w końcu zwrócić 429.
LIMITED=0
for attempt in $(seq 1 20); do
  request POST /api/staff/pickup "$K_JAR" "$K_CSRF" "{\"code\":\"BRAK$attempt\"}"
  if [ "$STATUS" = "429" ]; then
    LIMITED=1
    break
  fi
done
[ "$LIMITED" = "1" ] && pass "QR: limit nieudanych prób zwraca 429" \
  || fail "QR: limiter nie zadziałał po $attempt próbach (ostatni $STATUS)"

# Po zablokowaniu nawet poprawny kod jest odrzucany przez limiter.
request POST /api/staff/pickup "$K_JAR" "$K_CSRF" "{\"code\":\"$QR_TOKEN\"}"
[ "$STATUS" = "429" ] && pass "QR: blokada limitera obejmuje też poprawny kod" \
  || fail "QR: po blokadzie -> $STATUS ($BODY)"

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

request PATCH "/api/staff/orders/$ORDER4" "$K_JAR" "$K_CSRF" '{"payment_status":"paid"}'
[ "$STATUS" = "409" ] && pass "płatność: kuchnia nie opłaca anulowanego (409)" \
  || fail "płatność: anulowane -> $STATUS ($BODY)"

# ---------------------------------------------------------------- nieodebrane
request PUT /api/admin/settings "$ADMIN_JAR" "$ADMIN_CSRF" \
  '{"pickup_limit_minutes":1,"max_uncollected":1,"cancel_window_minutes":30}'
[ "$STATUS" = "200" ] && pass "ustawienia: limit odbioru i blokada zapisane" || fail "ustawienia: uncollected $STATUS"

create_order cash smoke-idem-0005
ORDER5=$(printf '%s' "$BODY" | jq -r '.id // empty')
request PATCH "/api/staff/orders/$ORDER5" "$K_JAR" "$K_CSRF" '{"status":"preparing"}'
request PATCH "/api/staff/orders/$ORDER5" "$K_JAR" "$K_CSRF" '{"status":"ready"}'
[ "$STATUS" = "200" ] && pass "nieodebrane: zamówienie gotowe" || fail "nieodebrane: ready $STATUS"
# Najpierw opłacone, potem nieodebrane — raport nie może policzyć go dwa razy.
request PATCH "/api/staff/orders/$ORDER5" "$K_JAR" "$K_CSRF" '{"payment_status":"paid"}'
PAID_AT_5=$(printf '%s' "$BODY" | jq -r '.paid_at // empty')
[ "$STATUS" = "200" ] && [ -n "$PAID_AT_5" ] \
  && pass "nieodebrane: zamówienie opłacone przed nieodebraniem" \
  || fail "nieodebrane: paid -> $STATUS at=$PAID_AT_5"
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
BLOCK_MSG=$(printf '%s' "$BODY" | jq -r '.error // .message // empty')
[ "$STATUS" = "409" ] \
  && printf '%s' "$BLOCK_MSG" | grep -qi "nieodebran" \
  && printf '%s' "$BLOCK_MSG" | grep -qi "blokad" \
  && pass "nieodebrane: blokada z komunikatem dla ucznia (409)" \
  || fail "nieodebrane: blokada -> $STATUS ($BLOCK_MSG)"

request PATCH "/api/staff/orders/$ORDER5" "$K_JAR" "$K_CSRF" '{"status":"preparing"}'
[ "$STATUS" = "409" ] && pass "nieodebrane: kuchnia nie zmienia zamkniętego (409)" || fail "nieodebrane: kuchnia -> $STATUS"
request PATCH "/api/staff/orders/$ORDER5" "$K_JAR" "$K_CSRF" '{"payment_status":"paid"}'
[ "$STATUS" = "409" ] && pass "nieodebrane: kuchnia nie opłaca nieodebranego (409)" \
  || fail "nieodebrane: płatność kuchni -> $STATUS ($BODY)"

# Osobna akcja administratora: zdejmij blokadę bez kasowania historii.
request POST "/api/admin/users/$S1_ID/clear-uncollected-block" "$K_JAR" "$K_CSRF" '{}'
[ "$STATUS" = "403" ] && pass "odblokowanie: tylko administrator (403)" \
  || fail "odblokowanie: kuchnia -> $STATUS ($BODY)"
request POST "/api/admin/users/$S1_ID/clear-uncollected-block" "$ADMIN_JAR" "$ADMIN_CSRF" '{}'
[ "$STATUS" = "200" ] && pass "odblokowanie: admin zdejmuje blokadę" \
  || fail "odblokowanie: admin -> $STATUS ($BODY)"
UNCOLLECTED_AFTER_UNBLOCK=$(db_scalar "SELECT uncollected_at FROM orders WHERE id=$ORDER5;")
[ "$UNCOLLECTED_AFTER_UNBLOCK" = "$UNCOLLECTED" ] \
  && pass "odblokowanie: historia nieodebranego bez zmian" \
  || fail "odblokowanie: znacznik=$UNCOLLECTED_AFTER_UNBLOCK oczekiwano $UNCOLLECTED"
AUDIT_UNBLOCK=$(db_scalar "SELECT COUNT(*) FROM audit_log WHERE action='uncollected_block_cleared';")
[ "$AUDIT_UNBLOCK" -ge 1 ] && pass "odblokowanie: wpis w audycie" \
  || fail "odblokowanie: brak audytu ($AUDIT_UNBLOCK)"

create_order cash smoke-idem-0007
[ "$STATUS" = "201" ] && pass "odblokowanie: nowe zamówienie znów możliwe" \
  || fail "odblokowanie: po akcji admina -> $STATUS ($BODY)"

# ---------------------------------------------------------------- raport i CSV
REPORT_DATE=$(TZ=Europe/Warsaw date +%F)
NEXT_DATE=$(TZ=Europe/Warsaw date -d "$REPORT_DATE +1 day" +%F)
NEXT_NEXT_DATE=$(TZ=Europe/Warsaw date -d "$NEXT_DATE +1 day" +%F)
DAY_START=$(TZ=Europe/Warsaw date -d "$REPORT_DATE 00:00" +%s)
DAY_END=$(TZ=Europe/Warsaw date -d "$NEXT_DATE 00:00" +%s)
NEXT_END=$(TZ=Europe/Warsaw date -d "$NEXT_NEXT_DATE 00:00" +%s)

# Granica doby: 23:59 należy do REPORT_DATE, 00:01 już do NEXT_DATE.
BOUND_BEFORE_AT=$((DAY_END - 60))
BOUND_AFTER_AT=$((DAY_END + 60))
BOUND_BEFORE_ID=$(db_scalar "INSERT INTO orders (user_id,code,qr_token,status,payment_method,payment_status,total_cents,pickup_time,created_at,updated_at,paid_at,paid_by) VALUES ($S1_ID,'BND001','bndtoken001','completed','cash','paid',$ITEM_PRICE,'23:59',$BOUND_BEFORE_AT,$BOUND_BEFORE_AT,$BOUND_BEFORE_AT,$DB_K_ID); SELECT last_insert_rowid();")
BOUND_AFTER_ID=$(db_scalar "INSERT INTO orders (user_id,code,qr_token,status,payment_method,payment_status,total_cents,pickup_time,created_at,updated_at,paid_at,paid_by) VALUES ($S1_ID,'BND002','bndtoken002','completed','cash','paid',$ITEM_PRICE,'00:01',$BOUND_AFTER_AT,$BOUND_AFTER_AT,$BOUND_AFTER_AT,$DB_K_ID); SELECT last_insert_rowid();")

request GET "/api/staff/orders?scope=today" "$K_JAR" "" ""
HAS_BEFORE=$(printf '%s' "$BODY" | jq --argjson id "$BOUND_BEFORE_ID" '[.[] | select(.id == $id)] | length')
HAS_AFTER=$(printf '%s' "$BODY" | jq --argjson id "$BOUND_AFTER_ID" '[.[] | select(.id == $id)] | length')
[ "$HAS_BEFORE" = "1" ] && [ "$HAS_AFTER" = "0" ] \
  && pass "strefa: dzisiejsze zawiera 23:59, pomija 00:01" \
  || fail "strefa: dzisiejsze 23:59=$HAS_BEFORE 00:01=$HAS_AFTER"

request GET "/api/staff/report?date=$REPORT_DATE" "$K_JAR" "" ""
REPORT_PAID=$(printf '%s' "$BODY" | jq -r '.paid_cents // empty')
REPORT_ORDERS=$(printf '%s' "$BODY" | jq -r '.orders // empty')
REPORT_PAID_ORDERS=$(printf '%s' "$BODY" | jq -r '.paid_orders // empty')
REPORT_UNPAID_ORDERS=$(printf '%s' "$BODY" | jq -r '.unpaid_orders // empty')
REPORT_CANCELLED=$(printf '%s' "$BODY" | jq -r '.cancelled_orders // empty')
REPORT_UNCOLLECTED=$(printf '%s' "$BODY" | jq -r '.uncollected_orders // empty')
REPORT_CASH=$(printf '%s' "$BODY" | jq -r '.cash.paid_cents // empty')
REPORT_CARD=$(printf '%s' "$BODY" | jq -r '.card.paid_cents // empty')
REPORT_CASH_ORDERS=$(printf '%s' "$BODY" | jq -r '.cash.orders // empty')
REPORT_CARD_ORDERS=$(printf '%s' "$BODY" | jq -r '.card.orders // empty')
REPORT_INVARIANT=$(printf '%s' "$BODY" | jq -r '.paid_orders + .unpaid_orders + .cancelled_orders + .uncollected_orders')
REPORT_CASH_INVARIANT=$(printf '%s' "$BODY" | jq -r '.cash.paid_orders + .cash.unpaid_orders + .cash.cancelled_orders + .cash.uncollected_orders')
REPORT_CARD_INVARIANT=$(printf '%s' "$BODY" | jq -r '.card.paid_orders + .card.unpaid_orders + .card.cancelled_orders + .card.uncollected_orders')

DB_ORDERS=$(db_scalar "SELECT COUNT(*) FROM orders WHERE created_at >= $DAY_START AND created_at < $DAY_END;")
DB_PAID=$(db_scalar "SELECT COALESCE(SUM(total_cents),0) FROM orders WHERE created_at >= $DAY_START AND created_at < $DAY_END AND payment_status='paid' AND NOT (status='cancelled' AND uncollected_at IS NULL);")
DB_PAID_ORDERS=$(db_scalar "SELECT COUNT(*) FROM orders WHERE created_at >= $DAY_START AND created_at < $DAY_END AND payment_status='paid' AND uncollected_at IS NULL AND NOT (status='cancelled' AND uncollected_at IS NULL);")
DB_UNPAID_ORDERS=$(db_scalar "SELECT COUNT(*) FROM orders WHERE created_at >= $DAY_START AND created_at < $DAY_END AND payment_status='unpaid' AND uncollected_at IS NULL AND NOT (status='cancelled' AND uncollected_at IS NULL);")
DB_CANCELLED=$(db_scalar "SELECT COUNT(*) FROM orders WHERE created_at >= $DAY_START AND created_at < $DAY_END AND status='cancelled' AND uncollected_at IS NULL;")
DB_CASH=$(db_scalar "SELECT COALESCE(SUM(total_cents),0) FROM orders WHERE created_at >= $DAY_START AND created_at < $DAY_END AND payment_method='cash' AND payment_status='paid' AND NOT (status='cancelled' AND uncollected_at IS NULL);")
DB_CARD=$(db_scalar "SELECT COALESCE(SUM(total_cents),0) FROM orders WHERE created_at >= $DAY_START AND created_at < $DAY_END AND payment_method='card' AND payment_status='paid' AND NOT (status='cancelled' AND uncollected_at IS NULL);")
DB_UNCOLLECTED=$(db_scalar "SELECT COUNT(*) FROM orders WHERE created_at >= $DAY_START AND created_at < $DAY_END AND uncollected_at IS NOT NULL;")

[ "$REPORT_ORDERS" = "$DB_ORDERS" ] && pass "raport: liczba zamówień zgodna z bazą" || fail "raport: orders=$REPORT_ORDERS db=$DB_ORDERS"
[ "$REPORT_PAID" = "$DB_PAID" ] && pass "raport: suma opłaconych zgodna z bazą" || fail "raport: paid=$REPORT_PAID db=$DB_PAID"
[ "$REPORT_PAID_ORDERS" = "$DB_PAID_ORDERS" ] && pass "raport: licznik opłaconych bez nieodebranych" || fail "raport: paid_orders=$REPORT_PAID_ORDERS db=$DB_PAID_ORDERS"
[ "$REPORT_UNPAID_ORDERS" = "$DB_UNPAID_ORDERS" ] && pass "raport: licznik nieopłaconych bez nieodebranych" || fail "raport: unpaid_orders=$REPORT_UNPAID_ORDERS db=$DB_UNPAID_ORDERS"
[ "$REPORT_CANCELLED" = "$DB_CANCELLED" ] && pass "raport: anulowane bez nieodebranych" || fail "raport: cancelled=$REPORT_CANCELLED db=$DB_CANCELLED"
[ "$REPORT_CASH" = "$DB_CASH" ] && pass "raport: gotówka zgodna z bazą" || fail "raport: cash=$REPORT_CASH db=$DB_CASH"
[ "$REPORT_CARD" = "$DB_CARD" ] && pass "raport: karta zgodna z bazą" || fail "raport: card=$REPORT_CARD db=$DB_CARD"
[ "$REPORT_UNCOLLECTED" = "$DB_UNCOLLECTED" ] && pass "raport: nieodebrane zgodne z bazą" || fail "raport: uncollected=$REPORT_UNCOLLECTED db=$DB_UNCOLLECTED"
[ "$REPORT_INVARIANT" = "$REPORT_ORDERS" ] && pass "raport: sumy kategorii = liczba zamówień" || fail "raport: suma=$REPORT_INVARIANT orders=$REPORT_ORDERS"
[ "$REPORT_CASH_INVARIANT" = "$REPORT_CASH_ORDERS" ] && [ "$REPORT_CARD_INVARIANT" = "$REPORT_CARD_ORDERS" ] \
  && pass "raport: sumy kategorii cash/card spójne" \
  || fail "raport: cash=$REPORT_CASH_INVARIANT/$REPORT_CASH_ORDERS card=$REPORT_CARD_INVARIANT/$REPORT_CARD_ORDERS"
[ $((REPORT_CASH_ORDERS + REPORT_CARD_ORDERS)) = "$REPORT_ORDERS" ] \
  && pass "raport: cash + card = razem" \
  || fail "raport: cash/card orders=$REPORT_CASH_ORDERS+$REPORT_CARD_ORDERS != $REPORT_ORDERS"

request GET "/api/staff/report?date=$NEXT_DATE" "$K_JAR" "" ""
NEXT_ORDERS=$(printf '%s' "$BODY" | jq -r '.orders // empty')
DB_NEXT=$(db_scalar "SELECT COUNT(*) FROM orders WHERE created_at >= $DAY_END AND created_at < $NEXT_END;")
[ "$NEXT_ORDERS" = "$DB_NEXT" ] && [ "$NEXT_ORDERS" -ge 1 ] \
  && pass "strefa: raport następnego dnia zawiera 00:01" \
  || fail "strefa: następny dzień=$NEXT_ORDERS db=$DB_NEXT"

CSV_HEADERS=$(curl -s --noproxy '*' -D - -o "$TMP/report.csv" -H "Host: $HOSTADDR" \
  -H "Origin: $ORIGIN" -b "$K_JAR" "http://$HOSTADDR/api/staff/report.csv?date=$REPORT_DATE" \
  | tr -d '\r')
CSV_CACHE=$(printf '%s' "$CSV_HEADERS" | sed -n 's/^[Cc]ache-[Cc]ontrol: //p')
[ "$CSV_CACHE" = "no-store" ] && pass "CSV: Cache-Control no-store" || fail "CSV: cache=$CSV_CACHE"
grep -q '"gotówka"' "$TMP/report.csv" && grep -q '"karta"' "$TMP/report.csv" && grep -q '"razem"' "$TMP/report.csv" \
  && pass "CSV: podział gotówka/karta/razem" \
  || fail "CSV: brak podziału metod"

# Wartość z formułą w eksporcie kont musi dostać apostrof.
db_scalar "INSERT INTO users (login, password_hash, role, display_code, active, must_change_password, created_at) VALUES ('=1+1','x','student','=CSV1',1,0,unixepoch());" >/dev/null
curl -s --noproxy '*' -o "$TMP/users.csv" -H "Host: $HOSTADDR" -H "Origin: $ORIGIN" \
  -b "$ADMIN_JAR" "http://$HOSTADDR/api/admin/users/export.csv"
grep -Fq "\"'=1+1\"" "$TMP/users.csv" \
  && pass "CSV: formuła w eksporcie kont zabezpieczona" \
  || fail "CSV: brak zabezpieczenia formuły"
db_scalar "DELETE FROM users WHERE login='=1+1';" >/dev/null

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

create_order cash smoke-idem-0008
[ "$STATUS" = "201" ] && pass "blokada: po korekcie zamówienie znów możliwe" || fail "blokada: po korekcie -> $STATUS ($BODY)"

request GET "/api/staff/report?date=$REPORT_DATE" "$K_JAR" "" ""
REPORT_PAID2=$(printf '%s' "$BODY" | jq -r '.paid_cents // empty')
REPORT_AFTER_UNCOLLECTED=$(printf '%s' "$BODY" | jq -r '.uncollected_orders // empty')
REPORT_AFTER_INVARIANT=$(printf '%s' "$BODY" | jq -r '.paid_orders + .unpaid_orders + .cancelled_orders + .uncollected_orders')
REPORT_AFTER_ORDERS=$(printf '%s' "$BODY" | jq -r '.orders // empty')
DB_PAID2=$(db_scalar "SELECT COALESCE(SUM(total_cents),0) FROM orders WHERE created_at >= $DAY_START AND created_at < $DAY_END AND payment_status='paid' AND NOT (status='cancelled' AND uncollected_at IS NULL);")
[ "$REPORT_PAID2" = "$DB_PAID2" ] && pass "raport: sumy nadal zgodne po korekcie" || fail "raport: po korekcie $REPORT_PAID2 != $DB_PAID2"
[ "$REPORT_AFTER_UNCOLLECTED" = "0" ] && pass "korekta: nieodebrane zniknęło z raportu" \
  || fail "korekta: uncollected=$REPORT_AFTER_UNCOLLECTED"
[ "$REPORT_AFTER_INVARIANT" = "$REPORT_AFTER_ORDERS" ] \
  && pass "korekta: suma kategorii nadal = liczba zamówień" \
  || fail "korekta: suma=$REPORT_AFTER_INVARIANT orders=$REPORT_AFTER_ORDERS"

printf '\nWynik: %d błędów\n' "$FAILS"
exit "$FAILS"
