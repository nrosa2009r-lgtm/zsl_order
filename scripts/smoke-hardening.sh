#!/usr/bin/env bash
# UWAGA: skrypt testowy. Używa wyłącznie danych testowych, losowo
# generowanych kont i izolowanej bazy w /tmp/opencode. Nie uruchamiaj go
# na prawdziwej instancji i nigdy nie używaj tych danych logowania w
# produkcji.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

BIN="$ROOT/target/debug/zsl-order"
BASE="/tmp/opencode/zsl-section2"
TMP="$BASE/tmp"
HOSTADDR="127.0.0.1:38191"
ORIGIN="http://$HOSTADDR"
FAILS=0

pass() { printf 'PASS  %s\n' "$*"; }
fail() { printf 'FAIL  %s\n' "$*"; FAILS=$((FAILS + 1)); }

STATUS=""
BODY=""

request() {
  local method=$1 path=$2 jar=$3 csrf=$4 data=$5 ctype=${6:-application/json}
  local args=(-s --noproxy '*' -X "$method" -H "Host: $HOSTADDR" -H "Origin: $ORIGIN"
    -D "$TMP/headers" -o "$TMP/body" -w '%{http_code}')
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

response_header() {
  tr -d '\r' <"$TMP/headers" | grep -i "^$1:" | tail -n1 | cut -d' ' -f2-
}

PIDS=()
cleanup() {
  for pid in "${PIDS[@]:-}"; do
    kill "$pid" 2>/dev/null || true
    wait "$pid" 2>/dev/null || true
  done
}
trap cleanup EXIT

rm -rf "$BASE"
mkdir -p "$BASE" "$TMP"
ln -s "$ROOT/frontend" "$BASE/frontend"
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
BIND_ADDR=127.0.0.1:38191
COOKIE_SECURE=false
ENV

(cd "$BASE" && exec env RUST_LOG=warn "$BIN" >server.log 2>&1) &
PIDS+=("$!")

for _ in $(seq 1 150); do
  if curl -s --noproxy '*' -o /dev/null -H "Host: $HOSTADDR" \
    "http://$HOSTADDR/api/healthz"; then
    break
  fi
  sleep 0.2
done

for _ in $(seq 1 100); do
  [ -f "$BASE/admin_start.txt" ] && break
  sleep 0.1
done
ADMIN_LOGIN=$(sed -n 's/^Login:  //p' "$BASE/admin_start.txt" | head -n1)
ADMIN_PASS=$(sed -n 's/^Hasło:  //p' "$BASE/admin_start.txt" | head -n1)

MODE=$(stat -c '%a' "$BASE/admin_start.txt" 2>/dev/null)
[ "$MODE" = "600" ] && pass "admin: admin_start.txt ma 0600" \
  || fail "admin: admin_start.txt ma ${MODE:-brak} zamiast 600"

if git -C "$ROOT" check-ignore -q admin_start.txt; then
  pass "admin: admin_start.txt jest w .gitignore"
else
  fail "admin: admin_start.txt nie jest w .gitignore"
fi

# Plik leży w katalogu roboczym, nie w serwowanym frontend/.
request GET /admin_start.txt "" "" ""
[ "$STATUS" = "404" ] && pass "admin: admin_start.txt nie jest serwowany (404)" \
  || fail "admin: admin_start.txt dostępny przez HTTP ($STATUS)"

# ---------------------------------------------------------------- healthz
request GET /api/healthz "" "" ""
[ "$STATUS" = "200" ] && pass "healthz: /api/healthz 200" || fail "healthz: /api/healthz $STATUS"
request GET /healthz "" "" ""
[ "$STATUS" = "200" ] && pass "healthz: /healthz 200" || fail "healthz: /healthz $STATUS"

# ---------------------------------------------------------------- jsQR (vendored)
request GET /kitchen "" "" ""
[ "$STATUS" = "200" ] && pass "jsQR: strona kuchni 200" || fail "jsQR: /kitchen $STATUS"
if printf '%s' "$BODY" | grep -qF 'vendor/jsQR.js'; then
  fail "jsQR: skrypt dołączony statycznie do HTML (ma być ładowany warunkowo)"
else
  pass "jsQR: skrypt nie jest dołączony statycznie do HTML"
fi

curl -s --noproxy '*' -D "$TMP/jsqr.headers" -o "$TMP/jsqr.js" \
  -H "Host: $HOSTADDR" "http://$HOSTADDR/vendor/jsQR.js" || true
JSQR_TYPE=$(tr -d '\r' <"$TMP/jsqr.headers" | sed -n 's/^[Cc]ontent-[Tt]ype: //p')
case "$JSQR_TYPE" in
  application/javascript*|text/javascript*)
    pass "jsQR: Content-Type=$JSQR_TYPE" ;;
  *)
    fail "jsQR: niepoprawny Content-Type=${JSQR_TYPE:-brak}" ;;
esac
if grep -qF 'jsQR v1.4.0' "$TMP/jsqr.js" 2>/dev/null; then
  pass "jsQR: plik serwowany lokalnie z nagłówkiem wersji"
else
  fail "jsQR: brak pliku lub nagłówka wersji"
fi

# ---------------------------------------------------------------- hasła startowe (frontend)
ADMIN_JS="$ROOT/frontend/admin.js"
if grep -Eq 'localStorage|sessionStorage|indexedDB' "$ADMIN_JS"; then
  fail "hasła: admin.js używa trwałego magazynu przeglądarki"
else
  pass "hasła: admin.js nie używa localStorage/sessionStorage/IndexedDB"
fi
if grep -qF 'Hasła nie będą dostępne po wyjściu' "$ADMIN_JS" \
  && grep -qF 'konta-start-' "$ADMIN_JS" \
  && grep -qF 'klasa/grupa' "$ADMIN_JS" \
  && grep -qF 'URL.revokeObjectURL' "$ADMIN_JS"; then
  pass "hasła: ostrzeżenie, nazwa pliku, nagłówek CSV i revoke URL obecne"
else
  fail "hasła: brak ostrzeżenia, nazwy pliku, nagłówka CSV lub revoke URL"
fi

# ---------------------------------------------------------------- admin login + policy
ADMIN_JAR="$TMP/admin.jar"
login "$ADMIN_LOGIN" "$ADMIN_PASS" "$ADMIN_JAR"
[ "$STATUS" = "200" ] && pass "admin: logowanie startowe" || fail "admin: logowanie $STATUS"
ADMIN_CSRF="$CSRF"

request GET /api/admin/users "$ADMIN_JAR" "" ""
CODE=$(printf '%s' "$BODY" | jq -r '.code // empty' 2>/dev/null)
[ "$STATUS" = "403" ] && [ "$CODE" = "password_change_required" ] \
  && pass "admin: pierwsze logowanie wymusza zmianę hasła" \
  || fail "admin: brak wymuszonej zmiany hasła ($STATUS/$CODE)"

request POST /api/auth/change-password "$ADMIN_JAR" "$ADMIN_CSRF" \
  '{"current_password":"'"$ADMIN_PASS"'","new_password":"abcdefghijk"}' 
[ "$STATUS" = "400" ] && pass "hasło: 11 znaków odrzucone dla admina" || fail "hasło: 11 znaków -> $STATUS"

request POST /api/auth/change-password "$ADMIN_JAR" "$ADMIN_CSRF" \
  '{"current_password":"'"$ADMIN_PASS"'","new_password":"administrator"}'
[ "$STATUS" = "400" ] && pass "hasło: popularne hasło odrzucone" || fail "hasło: popularne -> $STATUS"

request POST /api/auth/change-password "$ADMIN_JAR" "$ADMIN_CSRF" \
  '{"current_password":"'"$ADMIN_PASS"'","new_password":"'"$ADMIN_LOGIN"'"}'
[ "$STATUS" = "400" ] && pass "hasło: login jako hasło odrzucony" || fail "hasło: login jako hasło -> $STATUS"

ADMIN_NEW="NoweAdmin#2026x"
request POST /api/auth/change-password "$ADMIN_JAR" "$ADMIN_CSRF" \
  '{"current_password":"'"$ADMIN_PASS"'","new_password":"'"$ADMIN_NEW"'"}'
[ "$STATUS" = "200" ] && pass "hasło: poprawne 15 znaków przyjęte" || fail "hasło: poprawne -> $STATUS ($BODY)"
[ ! -f "$BASE/admin_start.txt" ] && pass "admin: admin_start.txt usunięty po zmianie hasła" \
  || fail "admin: admin_start.txt nadal istnieje po zmianie hasła"

request GET /api/admin/users "$ADMIN_JAR" "" ""
[ "$STATUS" = "200" ] && pass "admin: po zmianie hasła panel dostępny" \
  || fail "admin: po zmianie hasła panel -> $STATUS ($BODY)"

# ---------------------------------------------------------------- CSV import
IMPORT_OK=$'klasa;liczba\n1A;2\n2B;1\n'
request POST /api/admin/users/import "$ADMIN_JAR" "$ADMIN_CSRF" "$IMPORT_OK" "text/csv; charset=utf-8"
if [ "$STATUS" = "200" ]; then
  COUNT=$(printf '%s' "$BODY" | jq '.accounts | length')
  GROUPED=$(printf '%s' "$BODY" | jq '[.accounts[].group] | unique | length')
  [ "$COUNT" = "3" ] && pass "import: 3 konta z 2 grup" || fail "import: konta=$COUNT"
  [ "$GROUPED" = "2" ] && pass "import: grupa obecna w odpowiedzi" || fail "import: grupy=$GROUPED"
  IMPORT_PASS=$(printf '%s' "$BODY" | jq -r '.accounts[0].password')
  CACHE=$(curl -s --noproxy '*' -D - -o /dev/null -X POST \
    -H "Host: $HOSTADDR" -H "Origin: $ORIGIN" -b "$ADMIN_JAR" \
    -H "X-CSRF-Token: $ADMIN_CSRF" -H 'Content-Type: text/csv' \
    --data-binary $'9Z;1\n' "http://$HOSTADDR/api/admin/users/import" \
    | tr -d '\r' | sed -n 's/^[Cc]ache-[Cc]ontrol: //p')
  [ "$CACHE" = "no-store" ] && pass "import: Cache-Control no-store" || fail "import: Cache-Control=$CACHE"
else
  fail "import: status $STATUS ($BODY)"
fi

request POST /api/admin/users/import "$ADMIN_JAR" "$ADMIN_CSRF" $'1A;0\n' "text/csv"
[ "$STATUS" = "400" ] && pass "import: 1A;0 odrzucone" || fail "import: 1A;0 -> $STATUS"

request POST /api/admin/users/import "$ADMIN_JAR" "$ADMIN_CSRF" $'1A;10;20\n' "text/csv"
[ "$STATUS" = "400" ] && pass "import: zły format odrzucony" || fail "import: zły format -> $STATUS"

ACCOUNTS_FILE="$TMP/accounts.json"
request GET "/api/admin/users?q=s-1a-" "$ADMIN_JAR" "" ""
LISTED=$(printf '%s' "$BODY" | jq 'length')
[ "$LISTED" = "2" ] && pass "import: 2 konta klasy 1A w wyszukiwaniu" || fail "import: wyszukiwanie 1A=$LISTED"

# ---------------------------------------------------------------- student IDOR
# Ponownie pobierz dane uczniów z importu — potrzebne są hasła z odpowiedzi.
request POST /api/admin/users/import "$ADMIN_JAR" "$ADMIN_CSRF" $'3C;1\n4D;1\n' "text/csv"
S1_LOGIN=$(printf '%s' "$BODY" | jq -r '.accounts[0].login')
S1_PASS=$(printf '%s' "$BODY" | jq -r '.accounts[0].password')
S2_LOGIN=$(printf '%s' "$BODY" | jq -r '.accounts[1].login')
S2_PASS=$(printf '%s' "$BODY" | jq -r '.accounts[1].password')

CLASS_GROUP=$(sqlite3 "$BASE/baza.db" "SELECT class_group FROM users WHERE login='$S1_LOGIN';")
[ "$CLASS_GROUP" = "3C" ] && pass "import: class_group zapisane (3C)" \
  || fail "import: class_group=$CLASS_GROUP"

S1_JAR="$TMP/s1.jar"
login "$S1_LOGIN" "$S1_PASS" "$S1_JAR"
[ "$STATUS" = "200" ] && pass "uczeń 1: logowanie" || fail "uczeń 1: logowanie $STATUS"
S1_CSRF="$CSRF"
request GET /api/student/menu "$S1_JAR" "" ""
CODE=$(printf '%s' "$BODY" | jq -r '.code // empty')
[ "$STATUS" = "403" ] && [ "$CODE" = "password_change_required" ] \
  && pass "import: wymuszona zmiana hasła przed dostępem do API" \
  || fail "import: wymuszona zmiana hasła -> $STATUS/$CODE"
request POST /api/auth/change-password "$S1_JAR" "$S1_CSRF" \
  '{"current_password":"'"$S1_PASS"'","new_password":"UczenHaslo10"}'
[ "$STATUS" = "200" ] && pass "uczeń: 11-znakowe hasło przyjęte (min. 10)" || fail "uczeń: zmiana hasła $STATUS ($BODY)"

request GET /api/student/menu "$S1_JAR" "" ""
ITEM=$(printf '%s' "$BODY" | jq -r '.items[0].id // empty')
if [ -n "$ITEM" ]; then
  request POST /api/student/orders "$S1_JAR" "$S1_CSRF" \
    '{"items":[{"menu_item_id":'"$ITEM"',"quantity":1}],"payment_method":"cash","pickup_time":"12:30","idempotency_key":"smoke-order-1"}'
  ORDER_ID=$(printf '%s' "$BODY" | jq -r '.id // empty')
  [ "$STATUS" = "201" ] && pass "uczeń 1: złożono zamówienie" || fail "uczeń 1: zamówienie $STATUS ($BODY)"
else
  fail "uczeń 1: brak pozycji menu do testu IDOR"
fi

S2_JAR="$TMP/s2.jar"
login "$S2_LOGIN" "$S2_PASS" "$S2_JAR"
S2_CSRF="$CSRF"
request POST /api/auth/change-password "$S2_JAR" "$S2_CSRF" \
  '{"current_password":"'"$S2_PASS"'","new_password":"UczenHaslo20"}'
if [ -n "${ORDER_ID:-}" ]; then
  request POST "/api/student/orders/$ORDER_ID/cancel" "$S2_JAR" "$S2_CSRF" ""
  [ "$STATUS" = "404" ] && pass "IDOR: uczeń 2 nie anuluje cudzego zamówienia (404)" || fail "IDOR: cudze zamówienie -> $STATUS"
fi

# ---------------------------------------------------------------- RBAC matrix
request POST /api/admin/users/generate "$ADMIN_JAR" "$ADMIN_CSRF" '{"role":"kitchen","count":1}'
GEN_CACHE=$(response_header Cache-Control)
[ "$GEN_CACHE" = "no-store" ] && pass "generate: Cache-Control no-store" \
  || fail "generate: Cache-Control=${GEN_CACHE:-brak}"
K_LOGIN=$(printf '%s' "$BODY" | jq -r '.accounts[0].login')
K_PASS=$(printf '%s' "$BODY" | jq -r '.accounts[0].password')
K_JAR="$TMP/kitchen.jar"
login "$K_LOGIN" "$K_PASS" "$K_JAR"
[ "$STATUS" = "200" ] && pass "kuchnia: logowanie" || fail "kuchnia: logowanie $STATUS"
K_CSRF="$CSRF"
request POST /api/auth/change-password "$K_JAR" "$K_CSRF" \
  '{"current_password":"'"$K_PASS"'","new_password":"KuchniaHaslo12"}'
[ "$STATUS" = "200" ] && pass "kuchnia: zmiana hasła startowego" || fail "kuchnia: zmiana hasła $STATUS ($BODY)"

expect_forbidden() {
  local jar=$1 csrf=$2 label=$3 method=$4 path=$5
  request "$method" "$path" "$jar" "$csrf" ""
  [ "$STATUS" = "403" ] && pass "RBAC: $label $method $path -> 403" \
    || fail "RBAC: $label $method $path -> $STATUS"
}

ADMIN_ROUTES=(
  "GET /api/admin/stats"
  "GET /api/admin/stats/activity"
  "GET /api/admin/users"
  "GET /api/admin/users/export.csv"
  "POST /api/admin/users/import"
  "POST /api/admin/users/generate"
  "PATCH /api/admin/users/1"
  "DELETE /api/admin/users/1"
  "POST /api/admin/users/1/reset-password"
  "DELETE /api/admin/users/1/sessions"
  "GET /api/admin/settings"
  "PUT /api/admin/settings"
  "GET /api/admin/audit"
)
STAFF_ROUTES=(
  "GET /api/staff/menu"
  "POST /api/staff/menu"
  "PUT /api/staff/menu/1"
  "DELETE /api/staff/menu/1"
  "GET /api/staff/orders"
  "PATCH /api/staff/orders/1"
  "POST /api/staff/pickup"
)

for entry in "${ADMIN_ROUTES[@]}"; do
  expect_forbidden "$S1_JAR" "$S1_CSRF" "uczeń" ${entry%% *} ${entry#* }
  expect_forbidden "$K_JAR" "$K_CSRF" "kuchnia" ${entry%% *} ${entry#* }
done
for entry in "${STAFF_ROUTES[@]}"; do
  expect_forbidden "$S1_JAR" "$S1_CSRF" "uczeń" ${entry%% *} ${entry#* }
done

# Audyt importu/generowania: kto, ile kont, bez haseł.
request GET /api/admin/audit "$ADMIN_JAR" "" ""
AUDIT_HIT=$(printf '%s' "$BODY" | jq '[.[] | select(.action == "users_imported")] | length')
[ "$AUDIT_HIT" -ge 1 ] && pass "audyt: wpis users_imported obecny" || fail "audyt: brak users_imported"
GENERATED_HIT=$(printf '%s' "$BODY" | jq '[.[] | select(.action == "users_generated")] | length')
[ "$GENERATED_HIT" -ge 1 ] && pass "audyt: wpis users_generated obecny" || fail "audyt: brak users_generated"
printf '%s' "$BODY" | grep -qF -- "$IMPORT_PASS" \
  && fail "audyt: zawiera hasło z importu" \
  || pass "audyt: brak haseł z importu w logu audytu"
printf '%s' "$BODY" | grep -qF -- "$K_PASS" \
  && fail "audyt: zawiera wygenerowane hasło" \
  || pass "audyt: brak wygenerowanych haseł w logu audytu"

# Hasła nie mogą wyciekać w odpowiedziach endpointów innych niż
# generate/import. Eksport CSV metadanych również ich nie zawiera.
for endpoint in \
  "/api/admin/users" \
  "/api/admin/users/export.csv" \
  "/api/admin/audit" \
  "/api/admin/settings" \
  "/api/admin/stats"; do
  request GET "$endpoint" "$ADMIN_JAR" "" ""
  if printf '%s' "$BODY" | grep -qF -- "$IMPORT_PASS" \
    || printf '%s' "$BODY" | grep -qF -- "$K_PASS"; then
    fail "wyciek: $endpoint zawiera hasło startowe"
  else
    pass "wyciek: $endpoint bez haseł startowych"
  fi
done

# ---------------------------------------------------------------- pickup limit
for i in $(seq 1 8); do
  request POST /api/staff/pickup "$ADMIN_JAR" "$ADMIN_CSRF" '{"code":"ZZZZZZ"}'
done
request POST /api/staff/pickup "$ADMIN_JAR" "$ADMIN_CSRF" '{"code":"ZZZZZZ"}'
[ "$STATUS" = "429" ] && pass "pickup: 9. błędna próba zablokowana (429)" || fail "pickup: limit -> $STATUS"

# ---------------------------------------------------------------- session revoke
S1_ID=$(curl -s --noproxy '*' -b "$ADMIN_JAR" -H "Host: $HOSTADDR" \
  "http://$HOSTADDR/api/admin/users?q=$S1_LOGIN" | jq -r '.[0].id // empty')
if [ -n "$S1_ID" ]; then
  request DELETE "/api/admin/users/$S1_ID/sessions" "$ADMIN_JAR" "$ADMIN_CSRF" ""
  [ "$STATUS" = "200" ] && pass "sesje: unieważnienie sesji ucznia" || fail "sesje: unieważnienie $STATUS"
  request GET /api/auth/me "$S1_JAR" "" ""
  [ "$STATUS" = "401" ] && pass "sesje: stara sesja ucznia nie działa (401)" || fail "sesje: stara sesja -> $STATUS"
else
  fail "sesje: nie znaleziono id ucznia"
fi

# ---------------------------------------------------------------- login limiter per account
for i in $(seq 1 10); do
  login "s-nieistnieje" "zle-haslo-$i" "$TMP/nieistnieje.jar" >/dev/null 2>&1
done
login "s-nieistnieje" "zle-haslo-11" "$TMP/nieistnieje.jar"
[ "$STATUS" = "429" ] && pass "login: konto zablokowane po 10 błędach (429)" || fail "login: limit konta -> $STATUS"

# Hasła nie mogą trafić do logów technicznych (stdout -> journald
# w produkcji; w smoke trafia do server.log).
for password in "$IMPORT_PASS" "$K_PASS"; do
  if grep -rqF -- "$password" "$BASE/server.log" "$BASE/logs" 2>/dev/null; then
    fail "logi: zawierają hasło startowe"
  else
    pass "logi: brak hasła startowego w logach technicznych"
  fi
done

printf '\nWynik: %d błędów\n' "$FAILS"
exit "$FAILS"
