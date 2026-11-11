#!/usr/bin/env bash
# UWAGA: skrypt testowy. Tworzy izolowane bazy w /tmp/opencode i nie dotyka
# prawdziwych danych. Nie uruchamiaj go na produkcji.
#
# Testuje dwie ścieżki migracji sqlx:
#   1. pusta baza  -> pełny schemat + _sqlx_migrations,
#   2. baza "legacy" utworzona starym schematem (bez _sqlx_migrations)
#      -> dane pozostają, class_group uzupełnione z loginów.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN="$ROOT/target/debug/zsl_order"
BASE="/tmp/opencode/zsl-migrate-test"
FAILS=0

pass() { printf 'PASS  %s\n' "$*"; }
fail() { printf 'FAIL  %s\n' "$*"; FAILS=$((FAILS + 1)); }

[ -x "$BIN" ] || { echo "Najpierw: cargo build"; exit 1; }
command -v sqlite3 >/dev/null || { echo "Wymagany sqlite3"; exit 1; }

rm -rf "$BASE"
mkdir -p "$BASE/empty" "$BASE/legacy"

for dir in empty legacy; do
  port=$([ "$dir" = empty ] && echo 38201 || echo 38202)
  cat >"$BASE/$dir/config.json" <<JSON
{"db_url":"sqlite://baza.db","max_connections":4,"session_hours":12}
JSON
  cat >"$BASE/$dir/.env" <<ENV
DEPLOY_MODE=local
BIND_ADDR=127.0.0.1:$port
COOKIE_SECURE=false
ENV
done

sqlite3 "$BASE/legacy/baza.db" < "$ROOT/migrations/0001_baseline.sql"
sqlite3 "$BASE/legacy/baza.db" <<'SQL'
INSERT INTO users (login,password_hash,role,display_code,active,must_change_password,created_at)
VALUES ('s-1a-abcdefghij','hash','student','UCZ-1A-AAAAA',1,1,100),
       ('s-2b-klmnopqrst','hash','student','UCZ-2B-BBBBB',1,1,101),
       ('s-zzzzzzzzzz','hash','student','UCZ-CCCCC',1,0,102),
       ('k-kuchnia1234','hash','kitchen','KUCH-DDDDD',1,0,103);
INSERT INTO menu_items (name,description,price_cents,category,allergens,available,sort_order,created_at,updated_at)
VALUES ('Zupa','Test',500,'Zupy','',1,1,100,100);
INSERT INTO orders (user_id,code,qr_token,status,payment_method,payment_status,total_cents,pickup_time,created_at,updated_at)
VALUES (1,'ABC123','tok123','completed','cash','paid',500,'12:00',100,100);
INSERT INTO order_items (order_id,menu_item_id,name,price_cents,quantity) VALUES (1,1,'Zupa',500,1);
INSERT INTO settings (key,value) VALUES ('cafeteria_name','Testowa stołówka');
SQL

run_instance() {
  local dir=$1 port=$2
  (cd "$dir" && exec env RUST_LOG=warn "$BIN" >server.log 2>&1) &
  local pid=$!
  for _ in $(seq 1 150); do
    curl -s --noproxy '*' -o /dev/null "http://127.0.0.1:$port/api/healthz" && break
    sleep 0.2
  done
  kill "$pid" 2>/dev/null
  wait "$pid" 2>/dev/null
}

run_instance "$BASE/empty" 38201
run_instance "$BASE/legacy" 38202

empty_db="$BASE/empty/baza.db"
legacy_db="$BASE/legacy/baza.db"

[ "$(sqlite3 "$empty_db" 'SELECT COUNT(*) FROM _sqlx_migrations WHERE success = 1;')" = "4" ] \
  && pass "pusta baza: 4 migracje zastosowane" \
  || fail "pusta baza: brak migracji"

for table in users sessions menu_items orders order_items settings audit_log; do
  [ "$(sqlite3 "$empty_db" "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='$table';")" = "1" ] \
    && pass "pusta baza: tabela $table" \
    || fail "pusta baza: brak tabeli $table"
done

[ "$(sqlite3 "$legacy_db" 'SELECT COUNT(*) FROM orders;')" = "1" ] \
  && pass "legacy: zamówienie zachowane" \
  || fail "legacy: utracono zamówienie"

for column in paid_at paid_by ready_at uncollected_at idempotency_key; do
  [ "$(sqlite3 "$legacy_db" "SELECT COUNT(*) FROM pragma_table_info('orders') WHERE name='$column';")" = "1" ] \
    && pass "legacy: kolumna orders.$column" \
    || fail "legacy: brak kolumny orders.$column"
done

[ "$(sqlite3 "$legacy_db" "SELECT COUNT(*) FROM orders WHERE payment_status='paid' AND paid_at IS NOT NULL;")" = "1" ] \
  && pass "legacy: paid_at uzupełnione dla opłaconych" \
  || fail "legacy: brak paid_at dla opłaconego zamówienia"

[ "$(sqlite3 "$legacy_db" "SELECT COUNT(*) FROM users WHERE role != 'admin';")" = "4" ] \
  && pass "legacy: konta zachowane" \
  || fail "legacy: utracono konta"

[ "$(sqlite3 "$legacy_db" "SELECT class_group FROM users WHERE login='s-1a-abcdefghij';")" = "1A" ] \
  && pass "legacy: class_group 1A uzupełnione" \
  || fail "legacy: brak class_group 1A"

[ "$(sqlite3 "$legacy_db" "SELECT COUNT(*) FROM pragma_table_info('users') WHERE name='uncollected_block_reset_at';")" = "1" ] \
  && pass "legacy: kolumna users.uncollected_block_reset_at" \
  || fail "legacy: brak kolumny users.uncollected_block_reset_at"

[ "$(sqlite3 "$legacy_db" "SELECT class_group FROM users WHERE login='s-2b-klmnopqrst';")" = "2B" ] \
  && pass "legacy: class_group 2B uzupełnione" \
  || fail "legacy: brak class_group 2B"

[ -z "$(sqlite3 "$legacy_db" "SELECT class_group FROM users WHERE login='s-zzzzzzzzzz';")" ] \
  && pass "legacy: zwykłe konto bez class_group" \
  || fail "legacy: zwykłe konto dostało class_group"

[ "$(sqlite3 "$legacy_db" "SELECT value FROM settings WHERE key='cafeteria_name';")" = "Testowa stołówka" ] \
  && pass "legacy: ustawienia zachowane" \
  || fail "legacy: utracono ustawienia"

[ "$(sqlite3 "$legacy_db" 'PRAGMA integrity_check;')" = "ok" ] \
  && pass "legacy: integrity_check ok" \
  || fail "legacy: integrity_check nie ok"

printf '\nWynik: %d błędów\n' "$FAILS"
exit "$FAILS"
