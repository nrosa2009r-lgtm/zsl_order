#!/usr/bin/env bash
set -uo pipefail

API="http://127.0.0.1:38080"
DIR="/tmp/opencode/zsl-test"
BODY="$DIR/.body"
HDR="$DIR/.headers"
FAILS=0

ok() { printf 'PASS  %s\n' "$*"; }
bad() { printf 'FAIL  %s\n' "$*"; FAILS=$((FAILS + 1)); }

# req METHOD PATH JAR [JSON] [CSRF] [EXTRA...]
req() {
  local method=$1 path=$2
  shift 2
  local body_json=$1
  local csrf_token=$2
  shift 2
  local extra=$@
  local response
  response=$(curl -s -X "$method" "$API$path" \
    -H "Content-Type: application/json" \
    -H "Authorization: Bearer $csrf_token" \
    -d "$body_json" \
    ${extra:+-H "X-CSRF-Token: $csrf_token"} \
    -w "\n%{http_code}" \
    -o "$BODY")
  
  local http_code=$(echo "$response" | tail -n1 | tr -d ' ')
  local body_body=$(echo "$response" | sed '$d')
  
  if [ "$http_code" != "$3" ]; then
    echo "FAIL: Expected $3, got $http_code"
    echo "Response: $body_body"
    exit 1
  fi
  
  if [ "$5" != "empty" ] && [ -n "$body_body" ]; then
    echo "$body_body" | jq -e "$5" >/dev/null 2>&1 || (echo "FAIL: JSON parse error"; echo "$body_body"; exit 1)
  fi
  
  printf 'PASS  %s\n' "$*"
}

# Cleanup
rm -rf "$DIR"
mkdir -p "$DIR"
touch "$BODY" "$HDR"

# Login as admin (first time)
ADMIN_LOGIN="admin-test"
ADMIN_PW="AdminNowe#2026"
ADMIN_CSRF=$(printf '%s' "$(curl -s -X POST "$API/auth/login" -H "Content-Type: application/json" -d '{"login":"'$ADMIN_LOGIN'","password":"'$ADMIN_PW'"}' | jq -r '.csrf_token // empty')" || echo "")
if [ -z "$ADMIN_CSRF" ]; then
  echo "FAIL: Could not get CSRF token for admin login"
  exit 1
fi

# Test basic auth
req GET /api/auth/me "$AJ" "$ADMIN_CSRF" '{"login":"'$ADMIN_LOGIN'","password":"'$ADMIN_PW'"}' '{"login":"'$ADMIN_LOGIN'","role":"admin"}' empty

# ---------------------------------------------------------------- stats activity
req GET /api/admin/stats/activity?days=7 "$AJ" "$ADMIN_CSRF" '{"days":7}' empty
expect_status 200 "activity for 7 days"

req GET /api/admin/stats/activity?days=90 "$AJ" "$ADMIN_CSRF" '{"days":90}' empty
expect_status 200 "activity for 90 days"

# ---------------------------------------------------------------- users export
req GET /api/admin/users/export.csv "$AJ" "$ADMIN_CSRF" '{"role":"student"}' empty
expect_status 200 "CSV export"

# Check CSV content structure
if ! grep -q '^login;rola;kod;aktywne;haslo_startowe;utworzono;ostatnie_logowanie' "$BODY"; then
  echo "FAIL: CSV header missing"
  exit 1
fi

# Check no sensitive data in logs
LOG_FILE="/home/natan/Music/zsl_order/logs/zsl_order.log.2026-11-07"
if [ ! -f "$LOG_FILE" ]; then
  echo "FAIL: Log file not found"
  exit 1
fi

if grep -q "password\|token\|ip" "$LOG_FILE"; then
  echo "FAIL: Sensitive data found in logs"
  exit 1
fi

# ---------------------------------------------------------------- users delete
req DELETE /api/admin/users/1 "$AJ" "$ADMIN_CSRF" '{"login":"s-test-1"}' empty
expect_status 200 "delete user"

# Verify deletion
req GET /api/admin/users/1 "$AJ" "$ADMIN_CSRF" '{"login":"s-test-1"}' empty
expect_status 404 "user not found after deletion"

# Verify log file still exists and is clean
if [ ! -f "$LOG_FILE" ]; then
  echo "FAIL: Log file deleted"
  exit 1
fi

if grep -q "password\|token\|ip" "$LOG_FILE"; then
  echo "FAIL: Sensitive data found in logs after deletion"
  exit 1
fi

# Cleanup
rm -rf "$DIR"
echo "All tests passed ($FAILS failures)"
exit 0