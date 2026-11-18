#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
# UWAGA: skrypt testowy. Używa wyłącznie danych testowych i izolowanej bazy
# w /tmp/opencode. Nie uruchamiaj go na prawdziwej instancji.
#
# Sprawdza masowe usuwanie i dezaktywację kont: podgląd dwuetapowy,
# re-autoryzację hasłem admina, rate limit, ochronę własnego konta
# i aktywnych administratorów, 409 przy zmianie liczby kont oraz
# atomowość (bez „połowy” usuniętych kont).
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN="$ROOT/target/debug/zsl-order"
BASE="/tmp/opencode/zsl-bulk"
TMP="$BASE/tmp"
DB="$BASE/baza.db"
HOSTADDR="127.0.0.1:38194"
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
  "login_lockout_minutes": 15,
  "pickup_max_failures": 8,
  "pickup_ip_max_failures": 100,
  "pickup_window_minutes": 10,
  "pickup_lockout_minutes": 15
}
JSON
cat >"$BASE/.env" <<'ENV'
DEPLOY_MODE=local
BIND_ADDR=127.0.0.1:38194
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
ADMIN_ID=$(db_scalar "SELECT id FROM users WHERE login='$ADMIN_LOGIN';")

# ------------------------------------------------------- konta testowe (2 uczniów)
request POST /api/admin/users/import "$ADMIN_JAR" "$ADMIN_CSRF" $'9A;2\n' "text/csv"
S1_LOGIN=$(printf '%s' "$BODY" | jq -r '.accounts[0].login')
S1_PASS=$(printf '%s' "$BODY" | jq -r '.accounts[0].password')
S2_LOGIN=$(printf '%s' "$BODY" | jq -r '.accounts[1].login')
S2_PASS=$(printf '%s' "$BODY" | jq -r '.accounts[1].password')
[ -n "$S1_LOGIN" ] && [ -n "$S2_LOGIN" ] && pass "import: dwa konta uczniów" \
  || fail "import: brak kont ($STATUS/$BODY)"

S1_JAR="$TMP/s1.jar"
login "$S1_LOGIN" "$S1_PASS" "$S1_JAR"
S1_CSRF="$CSRF"
request POST /api/auth/change-password "$S1_JAR" "$S1_CSRF" \
  '{"current_password":"'"$S1_PASS"'","new_password":"UczenHaslo10"}'
[ "$STATUS" = "200" ] && pass "uczeń 1: zmiana hasła startowego" || fail "uczeń 1: zmiana $STATUS"
S1_ID=$(db_scalar "SELECT id FROM users WHERE login='$S1_LOGIN';")

S2_JAR="$TMP/s2.jar"
login "$S2_LOGIN" "$S2_PASS" "$S2_JAR"
S2_CSRF="$CSRF"
request POST /api/auth/change-password "$S2_JAR" "$S2_CSRF" \
  '{"current_password":"'"$S2_PASS"'","new_password":"UczenHaslo11"}'
[ "$STATUS" = "200" ] && pass "uczeń 2: zmiana hasła startowego" || fail "uczeń 2: zmiana $STATUS"
S2_ID=$(db_scalar "SELECT id FROM users WHERE login='$S2_LOGIN';")

request POST /api/admin/users/generate "$ADMIN_JAR" "$ADMIN_CSRF" '{"role":"student","count":2}'
S3_LOGIN=$(printf '%s' "$BODY" | jq -r '.accounts[0].login')
S3_PASS=$(printf '%s' "$BODY" | jq -r '.accounts[0].password')
S4_LOGIN=$(printf '%s' "$BODY" | jq -r '.accounts[1].login')
S4_PASS=$(printf '%s' "$BODY" | jq -r '.accounts[1].password')
[ -n "$S3_LOGIN" ] && [ -n "$S4_LOGIN" ] && pass "generate: kolejne dwa konta" \
  || fail "generate: brak kont ($STATUS/$BODY)"
S3_ID=$(db_scalar "SELECT id FROM users WHERE login='$S3_LOGIN';")
S4_ID=$(db_scalar "SELECT id FROM users WHERE login='$S4_LOGIN';")

S3_JAR="$TMP/s3.jar"
login "$S3_LOGIN" "$S3_PASS" "$S3_JAR"
S3_CSRF="$CSRF"
request POST /api/auth/change-password "$S3_JAR" "$S3_CSRF" \
  '{"current_password":"'"$S3_PASS"'","new_password":"UczenHaslo12"}'
[ "$STATUS" = "200" ] && pass "uczeń 3: zmiana hasła startowego" || fail "uczeń 3: zmiana $STATUS"

# ------------------------------------------------------- zamówienia uczniów (do historii)
request GET /api/student/menu "$S1_JAR" "" ""
ITEM=$(printf '%s' "$BODY" | jq -r '.items[0].id // empty')
[ -n "$ITEM" ] && pass "menu: pozycja testowa" || fail "menu: brak pozycji"

request POST /api/student/orders "$S1_JAR" "$S1_CSRF" \
  '{"items":[{"menu_item_id":'"$ITEM"',"quantity":1}],"payment_method":"cash","pickup_time":"12:30","idempotency_key":"bulk-order-0001"}'
O1=$(printf '%s' "$BODY" | jq -r '.id // empty')
[ "$STATUS" = "201" ] && pass "zamówienie ucznia 1 utworzone" || fail "zamówienie 1 -> $STATUS ($BODY)"

request POST /api/student/orders "$S2_JAR" "$S2_CSRF" \
  '{"items":[{"menu_item_id":'"$ITEM"',"quantity":1}],"payment_method":"cash","pickup_time":"12:30","idempotency_key":"bulk-order-0002"}'
O2=$(printf '%s' "$BODY" | jq -r '.id // empty')
[ "$STATUS" = "201" ] && pass "zamówienie ucznia 2 utworzone" || fail "zamówienie 2 -> $STATUS ($BODY)"

# ---------------------------------------------------------------- uprawnienia
request POST /api/admin/users/bulk-preview "$S1_JAR" "$S1_CSRF" '{"role":"student"}'
[ "$STATUS" = "403" ] && pass "uprawnienia: uczeń bez dostępu do akcji masowych (403)" \
  || fail "uprawnienia: uczeń -> $STATUS ($BODY)"

request POST /api/admin/users/bulk-preview "$ADMIN_JAR" "$ADMIN_CSRF" '{}'
[ "$STATUS" = "400" ] && pass "podgląd: wybór bez filtru odrzucony (400)" \
  || fail "podgląd: pusty wybór -> $STATUS ($BODY)"

# ---------------------------------------------------------------- podgląd (krok 1)
request POST /api/admin/users/bulk-preview "$ADMIN_JAR" "$ADMIN_CSRF" '{"role":"student"}'
PREVIEW_COUNT=$(printf '%s' "$BODY" | jq -r '.count // empty')
PREVIEW_IDS=$(printf '%s' "$BODY" | jq -c '.ids // empty')
PREVIEW_PROTECTED=$(printf '%s' "$BODY" | jq -c '.protected // empty')
[ "$STATUS" = "200" ] && [ "$PREVIEW_COUNT" = "4" ] \
  && pass "podgląd: filtr uczniów -> 4 konta" \
  || fail "podgląd: status=$STATUS count=$PREVIEW_COUNT ($BODY)"
[ "$PREVIEW_PROTECTED" = "[]" ] && pass "podgląd: brak kont chronionych w filtrze uczniów" \
  || fail "podgląd: chronione=$PREVIEW_PROTECTED"

# Filtr administratorów: ostatni aktywny admin (własne konto) jest chroniony
# i wykluczony z akcji — podgląd pokazuje go tylko jako chroniony.
request POST /api/admin/users/bulk-preview "$ADMIN_JAR" "$ADMIN_CSRF" '{"role":"admin"}'
PREVIEW_ADMIN_COUNT=$(printf '%s' "$BODY" | jq -r '.count // empty')
PREVIEW_ADMIN_PROTECTED=$(printf '%s' "$BODY" | jq -c '[.protected[] | select(.id == '"$ADMIN_ID"')] | length')
[ "$STATUS" = "200" ] && [ "$PREVIEW_ADMIN_COUNT" = "0" ] && [ "$PREVIEW_ADMIN_PROTECTED" = "1" ] \
  && pass "podgląd: własny admin chroniony, nie wchodzi do akcji" \
  || fail "podgląd: admin count=$PREVIEW_ADMIN_COUNT chroniony=$PREVIEW_ADMIN_PROTECTED ($BODY)"

# ---------------------------------------------------------------- krok 2: re-autoryzacja
request POST /api/admin/users/bulk-delete "$ADMIN_JAR" "$ADMIN_CSRF" \
  '{"ids":['"$S1_ID"','"$S2_ID"'],"expected_count":2,"password":"ZleHaslo!123"}'
[ "$STATUS" = "401" ] && pass "re-autoryzacja: błędne hasło odrzucone (401)" \
  || fail "re-autoryzacja: -> $STATUS ($BODY)"
USERS_AFTER_BAD=$(db_scalar "SELECT COUNT(*) FROM users WHERE id IN ($S1_ID,$S2_ID);")
[ "$USERS_AFTER_BAD" = "2" ] && pass "re-autoryzacja: błędne hasło nie usuwa kont" \
  || fail "re-autoryzacja: konta=$USERS_AFTER_BAD"

request POST /api/admin/users/bulk-delete "$ADMIN_JAR" "$ADMIN_CSRF" \
  '{"ids":['"$S1_ID"','"$S2_ID"'],"expected_count":2}'
[ "$STATUS" = "400" ] && pass "re-autoryzacja: brak hasła odrzucony (400)" \
  || fail "re-autoryzacja: brak hasła -> $STATUS ($BODY)"
[ "$STATUS" = "400" ] && printf '%s' "$BODY" | grep -qi "hasło" \
  && pass "re-autoryzacja: komunikat wskazuje na hasło" \
  || fail "re-autoryzacja: treść błędu brak hasła ($BODY)"

# ---------------------------------------------------------------- zmiana liczby kont (409)
request POST /api/admin/users/bulk-delete "$ADMIN_JAR" "$ADMIN_CSRF" \
  '{"ids":['"$S1_ID"','"$S2_ID"'],"expected_count":1,"password":"NoweAdmin#2026x"}'
[ "$STATUS" = "409" ] && pass "zgodność: mniejsza liczba kont niż w podglądzie -> 409" \
  || fail "zgodność: $STATUS ($BODY)"
request POST /api/admin/users/bulk-delete "$ADMIN_JAR" "$ADMIN_CSRF" \
  '{"ids":['"$S1_ID"','"$S2_ID"'],"expected_count":3,"password":"NoweAdmin#2026x"}'
[ "$STATUS" = "409" ] && pass "zgodność: większa liczba kont niż w podglądzie -> 409" \
  || fail "zgodność: $STATUS ($BODY)"
request POST /api/admin/users/bulk-delete "$ADMIN_JAR" "$ADMIN_CSRF" \
  '{"ids":['"$S1_ID"','"$S2_ID"'],"expected_count":0,"password":"NoweAdmin#2026x"}'
[ "$STATUS" = "400" ] && pass "zgodność: expected_count = 0 odrzucony (400)" \
  || fail "zgodność: expected_count=0 -> $STATUS ($BODY)"
USERS_AFTER_MISMATCH=$(db_scalar "SELECT COUNT(*) FROM users WHERE id IN ($S1_ID,$S2_ID);")
[ "$USERS_AFTER_MISMATCH" = "2" ] && pass "zgodność: niezgodność liczby nic nie zmienia" \
  || fail "zgodność: konta=$USERS_AFTER_MISMATCH"

# ---------------------------------------------------------------- ochrona kont
request POST /api/admin/users/bulk-delete "$ADMIN_JAR" "$ADMIN_CSRF" \
  '{"ids":['"$S1_ID"','"$ADMIN_ID"'],"expected_count":2,"password":"NoweAdmin#2026x"}'
PROTECTED_MSG=$(printf '%s' "$BODY" | jq -r '.error // .message // empty')
[ "$STATUS" = "409" ] && printf '%s' "$PROTECTED_MSG" | grep -qi "chronion" \
  && pass "ochrona: własne konto w liście ID -> 409 z komunikatem" \
  || fail "ochrona: $STATUS ($PROTECTED_MSG)"
SELF_STILL=$(db_scalar "SELECT COUNT(*) FROM users WHERE id=$ADMIN_ID;")
STUDENTS_STILL=$(db_scalar "SELECT COUNT(*) FROM users WHERE id IN ($S1_ID,$S2_ID);")
[ "$SELF_STILL" = "1" ] && [ "$STUDENTS_STILL" = "2" ] \
  && pass "ochrona: ani własne konto, ani uczniowie nie ruszeni" \
  || fail "ochrona: admin=$SELF_STILL uczniowie=$STUDENTS_STILL"

# ---------------------------------------------------------------- masowe usuwanie
# Podgląd z tą samą listą ID potwierdza rozstrzygnięty wybór przed krokiem 2.
request POST /api/admin/users/bulk-preview "$ADMIN_JAR" "$ADMIN_CSRF" \
  '{"ids":['"$S1_ID"','"$S2_ID"']}'
PREVIEW_IDS=$(printf '%s' "$BODY" | jq -c '.ids // empty')
[ "$STATUS" = "200" ] && [ "$PREVIEW_IDS" = "[$S1_ID,$S2_ID]" ] \
  && pass "podgląd: ta sama lista ID w kroku 1" \
  || fail "podgląd: ids=$PREVIEW_IDS ($BODY)"
request POST /api/admin/users/bulk-delete "$ADMIN_JAR" "$ADMIN_CSRF" \
  '{"ids":['"$S1_ID"','"$S2_ID"'],"expected_count":2,"password":"NoweAdmin#2026x"}'
BULK_COUNT=$(printf '%s' "$BODY" | jq -r '.count // empty')
BULK_ORPHANED=$(printf '%s' "$BODY" | jq -r '.orders_orphaned // empty')
[ "$STATUS" = "200" ] && [ "$BULK_COUNT" = "2" ] && [ "$BULK_ORPHANED" = "2" ] \
  && pass "usuwanie: 2 konta, 2 zamówienia zachowane (200)" \
  || fail "usuwanie: $STATUS count=$BULK_COUNT orphaned=$BULK_ORPHANED ($BODY)"

GONE=$(db_scalar "SELECT COUNT(*) FROM users WHERE id IN ($S1_ID,$S2_ID);")
[ "$GONE" = "0" ] && pass "usuwanie: konta usunięte z bazy" || fail "usuwanie: konta=$GONE"
OWNERS=$(db_scalar "SELECT COUNT(*) FROM orders WHERE id IN ($O1,$O2) AND user_id IS NULL;")
[ "$OWNERS" = "2" ] && pass "usuwanie: zamówienia zachowane bez powiązania z kontem (user_id NULL)" \
  || fail "usuwanie: zamówień z user_id NULL=$OWNERS (oczekiwano 2)"
KEPT=$(db_scalar "SELECT COUNT(*) FROM orders WHERE id IN ($O1,$O2);")
[ "$KEPT" = "2" ] && pass "usuwanie: historia zamówień kompletna" || fail "usuwanie: zamówień=$KEPT"
SESS_S1=$(db_scalar "SELECT COUNT(*) FROM sessions WHERE user_id=$S1_ID;")
[ "$SESS_S1" = "0" ] && pass "usuwanie: sesje usunięte (kaskada)" || fail "usuwanie: sesje=$SESS_S1"

# Sesja usuniętego ucznia przestaje działać.
request GET /api/student/orders "$S1_JAR" "" ""
[ "$STATUS" = "401" ] && pass "usuwanie: sesja usuniętego ucznia odrzucona (401)" \
  || fail "usuwanie: sesja -> $STATUS ($BODY)"

# Zamówienie bez konta pokazuje „konto usunięte”.
request GET "/api/staff/orders?scope=all" "$ADMIN_JAR" "" ""
ORDERS_STATUS="$STATUS"
DISPLAY_CODE=$(printf '%s' "$BODY" | jq -r --argjson id "$O1" '.[] | select(.id == $id) | .student_code // "BRAK"')
ORDER_FOUND=$(printf '%s' "$BODY" | jq --argjson id "$O1" '[.[] | select(.id == $id)] | length')
[ "$ORDERS_STATUS" = "200" ] && [ "$ORDER_FOUND" = "1" ] && [ "$DISPLAY_CODE" = "konto usunięte" ] \
  && pass "usuwanie: UI zamówień pokazuje „konto usunięte”" \
  || fail "usuwanie: status=$ORDERS_STATUS znaleziono=$ORDER_FOUND student_code=$DISPLAY_CODE"

AUDIT_DEL=$(db_scalar "SELECT COUNT(*) FROM audit_log WHERE action='bulk_users_deleted';")
[ "$AUDIT_DEL" -ge 1 ] && pass "usuwanie: zbiorczy wpis w audycie" || fail "usuwanie: brak audytu"
AUDIT_ACTOR=$(db_scalar "SELECT COUNT(*) FROM audit_log WHERE action='user_deleted' AND actor_id IS NULL;")
[ "$AUDIT_ACTOR" = "0" ] && pass "audyt: wpis zachowuje aktora" || fail "audyt: aktor zgubiony"

# ---------------------------------------------------------------- tryb filtru w kroku 2
request POST /api/admin/users/bulk-deactivate "$ADMIN_JAR" "$ADMIN_CSRF" \
  '{"role":"student","active":true,"expected_count":9,"password":"NoweAdmin#2026x"}'
[ "$STATUS" = "409" ] && pass "filtr: niezgodna liczba w trybie filtru -> 409" \
  || fail "filtr: $STATUS ($BODY)"

request POST /api/admin/users/bulk-deactivate "$ADMIN_JAR" "$ADMIN_CSRF" \
  '{"role":"student","active":true,"expected_count":2,"password":"NoweAdmin#2026x"}'
DEACT_COUNT=$(printf '%s' "$BODY" | jq -r '.count // empty')
[ "$STATUS" = "200" ] && [ "$DEACT_COUNT" = "2" ] \
  && pass "dezaktywacja: filtr zgodny z podglądem -> 2 konta" \
  || fail "dezaktywacja: $STATUS count=$DEACT_COUNT ($BODY)"

ACTIVE_LEFT=$(db_scalar "SELECT COUNT(*) FROM users WHERE id IN ($S3_ID,$S4_ID) AND active=1;")
[ "$ACTIVE_LEFT" = "0" ] && pass "dezaktywacja: konta nieaktywne" || fail "dezaktywacja: active=$ACTIVE_LEFT"
STILL_THERE=$(db_scalar "SELECT COUNT(*) FROM users WHERE id IN ($S3_ID,$S4_ID);")
[ "$STILL_THERE" = "2" ] && pass "dezaktywacja: konta istnieją w bazie" || fail "dezaktywacja: konta=$STILL_THERE"
SESS_S3=$(db_scalar "SELECT COUNT(*) FROM sessions WHERE user_id=$S3_ID;")
[ "$SESS_S3" = "0" ] && pass "dezaktywacja: sesje usunięte" || fail "dezaktywacja: sesje=$SESS_S3"
S3_ORDERS=$(db_scalar "SELECT COUNT(*) FROM orders WHERE user_id=$S3_ID;")
login "$S3_LOGIN" "$S3_PASS" "$TMP/s3b.jar"
[ "$STATUS" != "200" ] && pass "dezaktywacja: nie można zalogować się na konto (401)" \
  || fail "dezaktywacja: logowanie $STATUS"

AUDIT_DEACT=$(db_scalar "SELECT COUNT(*) FROM audit_log WHERE action='bulk_users_deactivated';")
[ "$AUDIT_DEACT" -ge 1 ] && pass "dezaktywacja: zbiorczy wpis w audycie" || fail "dezaktywacja: brak audytu"

# ---------------------------------------------------------------- rate limit (osobne konto admina)
request POST /api/admin/users/generate "$ADMIN_JAR" "$ADMIN_CSRF" '{"role":"admin","count":1}'
A2_LOGIN=$(printf '%s' "$BODY" | jq -r '.accounts[0].login')
A2_PASS=$(printf '%s' "$BODY" | jq -r '.accounts[0].password')
[ -n "$A2_LOGIN" ] && pass "rate limit: drugie konto administratora" || fail "rate limit: brak konta ($STATUS)"
A2_ID=$(db_scalar "SELECT id FROM users WHERE login='$A2_LOGIN';")

A2_JAR="$TMP/a2.jar"
login "$A2_LOGIN" "$A2_PASS" "$A2_JAR"
A2_CSRF="$CSRF"
request POST /api/auth/change-password "$A2_JAR" "$A2_CSRF" \
  '{"current_password":"'"$A2_PASS"'","new_password":"DrugiAdmin#2026x"}'
[ "$STATUS" = "200" ] && pass "rate limit: drugi admin zmienia hasło" || fail "rate limit: zmiana $STATUS"

LIMITED=0
WRONG_OK=0
for attempt in $(seq 1 8); do
  PAYLOAD='{"ids":['"$S3_ID"'],"expected_count":1,"password":"BledneHaslo'"$attempt"'"}'
  request POST /api/admin/users/bulk-delete "$A2_JAR" "$A2_CSRF" "$PAYLOAD"
  if [ "$STATUS" = "429" ]; then
    LIMITED=1
    break
  fi
  [ "$STATUS" = "401" ] && WRONG_OK=$((WRONG_OK + 1))
done
[ "$LIMITED" = "1" ] && [ "$WRONG_OK" -ge 4 ] \
  && pass "rate limit: po 5 błędnych hasłach -> 429 (401 x$WRONG_OK przedtem)" \
  || fail "rate limit: 429=$LIMITED 401=$WRONG_OK (ostatni $STATUS)"

# Nawet poprawne hasło jest odrzucane w oknie blokady.
request POST /api/admin/users/bulk-delete "$A2_JAR" "$A2_CSRF" \
  '{"ids":['"$S3_ID"'],"expected_count":1,"password":"DrugiAdmin#2026x"}'
[ "$STATUS" = "429" ] && pass "rate limit: blokada obejmuje też poprawne hasło" \
  || fail "rate limit: po blokadzie -> $STATUS ($BODY)"

# Pierwszy administrator nie jest zablokowany blokadą drugiego.
request POST /api/admin/users/bulk-preview "$ADMIN_JAR" "$ADMIN_CSRF" '{"role":"student"}'
[ "$STATUS" = "200" ] && pass "rate limit: pierwszy admin nadal działa" \
  || fail "rate limit: pierwszy admin -> $STATUS ($BODY)"

# ---------------------------------------------------------------- usuwanie pojedyncze
request POST /api/admin/users/generate "$ADMIN_JAR" "$ADMIN_CSRF" '{"role":"student","count":1}'
S5_LOGIN=$(printf '%s' "$BODY" | jq -r '.accounts[0].login')
S5_PASS=$(printf '%s' "$BODY" | jq -r '.accounts[0].password')
S5_ID=$(db_scalar "SELECT id FROM users WHERE login='$S5_LOGIN';")
[ -n "$S5_LOGIN" ] && pass "pojedyncze: konto testowe wygenerowane" || fail "pojedyncze: brak konta"
S5_JAR="$TMP/s5.jar"
login "$S5_LOGIN" "$S5_PASS" "$S5_JAR"
S5_CSRF="$CSRF"
request POST /api/auth/change-password "$S5_JAR" "$S5_CSRF" \
  '{"current_password":"'"$S5_PASS"'","new_password":"UczenHaslo15"}'
[ "$STATUS" = "200" ] && pass "uczeń 5: zmiana hasła startowego" || fail "uczeń 5: zmiana $STATUS"
request POST /api/student/orders "$S5_JAR" "$S5_CSRF" \
  '{"items":[{"menu_item_id":'"$ITEM"',"quantity":1}],"payment_method":"cash","pickup_time":"12:30","idempotency_key":"bulk-order-0005"}'
O5=$(printf '%s' "$BODY" | jq -r '.id // empty')
[ "$STATUS" = "201" ] && pass "zamówienie ucznia 5 utworzone" || fail "zamówienie 5 -> $STATUS ($BODY)"

request DELETE "/api/admin/users/$S5_ID" "$ADMIN_JAR" "$ADMIN_CSRF" ""
SINGLE_KEPT=$(printf '%s' "$BODY" | jq -r '.kept_orders // empty')
[ "$STATUS" = "200" ] && [ "$SINGLE_KEPT" = "1" ] \
  && pass "pojedyncze: usunięcie konta zachowuje zamówienie (kept_orders=1)" \
  || fail "pojedyncze: $STATUS kept=$SINGLE_KEPT ($BODY)"
SINGLE_OWNER=$(db_scalar "SELECT user_id FROM orders WHERE id=$O5;")
[ "$SINGLE_OWNER" = "" ] && pass "pojedyncze: zamówienie bez powiązania z kontem (NULL)" \
  || fail "pojedyncze: user_id=$SINGLE_OWNER"

# Własne konto nadal nie da się usunąć.
request DELETE "/api/admin/users/$ADMIN_ID" "$ADMIN_JAR" "$ADMIN_CSRF" ""
[ "$STATUS" = "400" ] && pass "ochrona: usunięcie własnego konta odrzucone (400)" \
  || fail "ochrona: własny delete -> $STATUS ($BODY)"

printf '\nWynik: %d błędów\n' "$FAILS"
exit "$FAILS"
