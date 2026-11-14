#!/bin/sh
# SPDX-License-Identifier: MIT
# Przygotowuje katalog roboczy i uruchamia aplikację.
#
# Aplikacja serwuje statyczne pliki z `frontend/` względem katalogu roboczego,
# a katalogiem roboczym jest /data (wolumen z bazą). Frontend leży w obrazie
# pod /app/frontend — dowiązujemy go, zamiast kopiować do wolumenu.
set -eu

if [ ! -e ./frontend ]; then
    ln -s /app/frontend ./frontend
fi

if [ ! -w . ]; then
    echo "Błąd: katalog roboczy $(pwd) nie jest zapisywalny dla użytkownika $(id -u)." >&2
    echo "Wolumen lub katalog montowany z hosta musi należeć do UID 10001 (zsl-order)." >&2
    exit 1
fi

# Kilka wskazówek w logach przy typowych problemach z siecią.
bind="${BIND_ADDR:-0.0.0.0:3000}"
case "$bind" in
    127.0.0.1:*|"[::1]":*|localhost:*)
        echo "Uwaga: BIND_ADDR=$bind nasłuchuje tylko na pętli — inne urządzenia się nie połączą." >&2
        ;;
esac

exec "$@"
