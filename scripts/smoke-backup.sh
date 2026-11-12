#!/usr/bin/env bash
# UWAGA: skrypt testowy. Tworzy izolowaną bazę w /tmp/opencode i nie
# dotyka prawdziwych danych. Wymaga zbudowanej binarki (`cargo build`)
# oraz sqlite3.
#
# Testuje: kopię spójną z WAL, uprawnienia 0600, integrity_check,
# --verify, retencję z minimalnymi 7 kopiami i obsługę błędów CLI.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN="$ROOT/target/debug/zsl-order"
BASE="/tmp/opencode/zsl-backup-test"
PORT=38351
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
command -v sqlite3 >/dev/null || { echo "Wymagany sqlite3"; exit 1; }

rm -rf "$BASE"
mkdir -p "$BASE"
ln -s "$ROOT/frontend" "$BASE/frontend"

cat >"$BASE/config.json" <<'JSON'
{
  "db_url": "sqlite://baza.db",
  "max_connections": 4,
  "session_hours": 12,
  "backup_dir": "backups",
  "backup_keep_days": 30
}
JSON

cat >"$BASE/.env" <<ENV
DEPLOY_MODE=local
BIND_ADDR=127.0.0.1:$PORT
TRUSTED_PROXIES=127.0.0.1,::1
COOKIE_SECURE=false
ENV

# Uruchom prawdziwą aplikację, aby baza miała dane i plik WAL.
(cd "$BASE" && exec env RUST_LOG=warn "$BIN" >server.log 2>&1) &
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
fi

[ -f "$BASE/admin_start.txt" ] && pass "start: baza i konto startowe utworzone" \
  || fail "start: brak admin_start.txt"

if [ -f "$BASE/baza.db-wal" ]; then
  pass "WAL: plik baza.db-wal istnieje w trakcie pracy"
else
  fail "WAL: brak pliku baza.db-wal"
fi

DB_MODE="$(stat -c '%a' "$BASE/baza.db")"
[ "$DB_MODE" = "600" ] && pass "uprawnienia: baza.db ma 0600" \
  || fail "uprawnienia: baza.db ma $DB_MODE zamiast 600"

JOURNAL="$(sqlite3 "$BASE/baza.db" 'PRAGMA journal_mode;' 2>/dev/null)"
[ "$JOURNAL" = "wal" ] && pass "WAL: journal_mode = wal" \
  || fail "WAL: journal_mode = ${JOURNAL:-błąd}"

# --------------------------------------------------------------- backup
if (cd "$BASE" && "$BIN" backup >backup-1.log 2>&1); then
  pass "backup: pierwsza kopia zakończona sukcesem"
else
  fail "backup: pierwsze uruchomienie nie powiodło się"
  cat "$BASE/backup-1.log"
fi

LATEST="$(ls -1t "$BASE"/backups/zsl-order-*.db 2>/dev/null | head -n1)"
if [ -n "$LATEST" ] && [ -f "$LATEST" ]; then
  pass "backup: plik kopii istnieje"
else
  fail "backup: brak pliku kopii"
fi

if [ -n "$LATEST" ]; then
  MODE="$(stat -c '%a' "$LATEST")"
  [ "$MODE" = "600" ] && pass "backup: uprawnienia kopii 0600" \
    || fail "backup: uprawnienia kopii $MODE != 600"

  [ ! -e "$LATEST-wal" ] && [ ! -e "$LATEST-shm" ] \
    && pass "backup: kopia jest samodzielnym plikiem (bez WAL/SHM)" \
    || fail "backup: obok kopii powstały pliki WAL/SHM"

  CHECK="$(sqlite3 "$LATEST" 'PRAGMA integrity_check;' 2>/dev/null)"
  [ "$CHECK" = "ok" ] && pass "backup: integrity_check = ok" \
    || fail "backup: integrity_check = ${CHECK:-błąd}"

  USERS="$(sqlite3 "$LATEST" 'SELECT COUNT(*) FROM users;' 2>/dev/null)"
  [ "${USERS:-0}" -ge 1 ] && pass "backup: kopia zawiera dane aplikacji" \
    || fail "backup: kopia nie zawiera tabeli users"

  if (cd "$BASE" && "$BIN" backup --verify "$LATEST" >verify-ok.log 2>&1); then
    pass "backup --verify: poprawna kopia przechodzi"
  else
    fail "backup --verify: poprawna kopia odrzucona"
    cat "$BASE/verify-ok.log"
  fi
fi

printf 'to nie jest baza sqlite' >"$BASE/bad.db"
if (cd "$BASE" && "$BIN" backup --verify "$BASE/bad.db" >verify-bad.log 2>&1); then
  fail "backup --verify: uszkodzony plik został zaakceptowany"
else
  pass "backup --verify: uszkodzony plik odrzucony"
fi

HELP="$(cd "$BASE" && "$BIN" backup --help 2>&1)"
case "$HELP" in
  *"VACUUM INTO"*) pass "backup --help: opisuje mechanizm kopii" ;;
  *) fail "backup --help: brak opisu VACUUM INTO" ;;
esac

if (cd "$BASE" && "$BIN" backup --nieznany >/dev/null 2>&1); then
  fail "backup: nieznany argument został przyjęty"
else
  pass "backup: nieznany argument kończy się błędem"
fi

# ------------------------------------------------------------ retencja
for i in $(seq -w 1 10); do
  touch -d '2020-01-01' "$BASE/backups/zsl-order-2020-01-${i}.db"
done
(cd "$BASE" && "$BIN" backup --keep-days 0 >backup-retention.log 2>&1)
COUNT="$(find "$BASE/backups" -maxdepth 1 -name 'zsl-order-*.db' | wc -l | tr -d ' ')"
[ "$COUNT" = "7" ] && pass "retencja: --keep-days 0 zachowuje 7 najnowszych" \
  || fail "retencja: pozostało $COUNT plików zamiast 7"

NEWEST="$(ls -1t "$BASE"/backups/zsl-order-*.db | head -n1)"
CHECK="$(sqlite3 "$NEWEST" 'PRAGMA integrity_check;' 2>/dev/null)"
[ "$CHECK" = "ok" ] && pass "retencja: najnowsza kopia nadal poprawna" \
  || fail "retencja: najnowsza kopia uszkodzona (${CHECK:-błąd})"

# --------------------------------------------------- nadpisanie BACKUP_DIR
if (cd "$BASE" && BACKUP_DIR=backups-env "$BIN" backup >backup-env.log 2>&1); then
  ENV_COUNT="$(find "$BASE/backups-env" -maxdepth 1 -name 'zsl-order-*.db' 2>/dev/null | wc -l | tr -d ' ')"
  [ "$ENV_COUNT" = "1" ] && pass "BACKUP_DIR: zmienna środowiskowa nadpisuje config.json" \
    || fail "BACKUP_DIR: liczba kopii $ENV_COUNT != 1"
else
  fail "BACKUP_DIR: backup z nadpisaniem nie powiódł się"
  cat "$BASE/backup-env.log"
fi

# ------------------------------------------------------ brak bazy źródłowej
mkdir -p "$BASE/missing"
cat >"$BASE/missing/config.json" <<'JSON'
{"db_url": "sqlite://nie-ma.db", "backup_dir": "backups"}
JSON
if (cd "$BASE/missing" && "$BIN" backup >missing.log 2>&1); then
  fail "błąd: backup bez bazy źródłowej zakończył się sukcesem"
else
  pass "błąd: backup bez bazy źródłowej kończy się błędem"
fi
grep -qi 'nie znaleziono bazy' "$BASE/missing/missing.log" \
  && pass "błąd: czytelny komunikat o braku bazy" \
  || fail "błąd: brak czytelnego komunikatu o braku bazy"

printf '\nWynik: %d błędów\n' "$FAILS"
exit "$FAILS"
