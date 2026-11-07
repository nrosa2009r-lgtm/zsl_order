# Wdrożenie i utrzymanie — ZSL Order

Dokument opisuje produkcyjne wdrożenie aplikacji na Linuxie, konfigurację
TLS, usługę systemd, backup, aktualizacje, monitoring i test dymny.
Mechanizmy ochrony opisuje [SECURITY.md](SECURITY.md).

## 1. Wymagania

- Linux (zalecany Debian/Ubuntu/Arch), systemd, Nginx lub Caddy.
- Rust 1.85+ tylko do budowania; na serwerze wystarczy binarka.
- SQLite w wersji wbudowanej w `sqlx` — nie trzeba instalować serwera DB.
- Osobny użytkownik systemowy, np. `zsl`.
- Ważny certyfikat TLS (Let's Encrypt lub wewnętrzny CA).
- Otwarty tylko port 443 (i 80 na redirect/ACME); port aplikacji
  dostępny wyłącznie lokalnie.

## 2. Budowanie

Na maszynie deweloperskiej lub w CI:

```bash
cargo build --release --locked
```

Binarka: `target/release/zsl_order`. Profil release ma LTO, `strip`,
`panic = "abort"` i optymalizację `opt-level = 3`.

Testy przed wdrożeniem:

```bash
cargo test --locked
cargo clippy --all-targets
```

## 3. Układ katalogów

```text
/opt/zsl_order/
├── zsl_order              # binarka (0755, root:root)
├── frontend/              # statyczne HTML/CSS/JS (0755, root:root)
└── data/                  # katalog roboczy usługi (0700, zsl:zsl)
    ├── config.json        # 0600
    ├── baza.db            # 0600
    ├── baza.db-wal        # tworzone przez SQLite
    ├── baza.db-shm
    └── admin_start.txt    # tylko przy pierwszym uruchomieniu, 0600
```

Aplikacja używa **ścieżek względnych** (`config.json`, `baza.db`,
`admin_start.txt`, `frontend/`) wobec katalogu roboczego. Dlatego
`WorkingDirectory` musi wskazywać `data/`, a `frontend/` musi być
dostępny względem tego katalogu. Można użyć symlinku:

```bash
ln -s /opt/zsl_order/frontend /opt/zsl_order/data/frontend
```

albo skopiować `frontend/` do `data/`. Symlink jest wygodniejszy przy
aktualizacjach.

### Instalacja

```bash
sudo useradd --system --home /opt/zsl_order --shell /usr/sbin/nologin zsl
sudo install -d -o zsl -g zsl -m 0700 /opt/zsl_order/data
sudo install -o root -g root -m 0755 target/release/zsl_order /opt/zsl_order/zsl_order
sudo cp -r frontend /opt/zsl_order/frontend
sudo chown -R root:root /opt/zsl_order/frontend
sudo ln -s /opt/zsl_order/frontend /opt/zsl_order/data/frontend
```

## 4. Konfiguracja

```bash
sudo -u zsl cp /opt/zsl_order/config.example.json /opt/zsl_order/data/config.json
sudo chmod 600 /opt/zsl_order/data/config.json
sudo -u zsl editor /opt/zsl_order/data/config.json
```

Produkcyjnie:

```json
{
  "ip": "127.0.0.1",
  "port": 3000,
  "max_connections": 16,
  "db_url": "sqlite://baza.db",
  "session_hours": 12,
  "secure_cookies": true
}
```

| Pole | Znaczenie | Produkcja |
| --- | --- | --- |
| `ip` | interfejs nasłuchu | `127.0.0.1` za proxy |
| `port` | port HTTP | dowolny wolny, np. 3000 |
| `max_connections` | pula SQLite | 8–16 |
| `db_url` | baza | `sqlite://baza.db` |
| `session_hours` | czas sesji | 8–12 |
| `secure_cookies` | `Secure` + HSTS + CSP upgrade | **`true`** |

**Uwaga:** `secure_cookies = true` wymaga HTTPS. Przy testach po HTTP
ustaw `false`, inaczej przeglądarka nie wyśle ciasteczka.

## 5. Pierwsze uruchomienie

Najbezpieczniej uruchomić aplikację raz w konsoli jako użytkownik usługi,
odebrać dane admina i od razu przejść na usługę:

```bash
cd /opt/zsl_order/data
sudo -u zsl /opt/zsl_order/zsl_order
```

W konsoli pojawią się login i hasło startowego administratora. Te same
dane są w `admin_start.txt` (`0600`). Następnie:

1. Zatrzymaj proces (`Ctrl+C`).
2. Uruchom usługę systemd.
3. Zaloguj się do `/admin` danymi startowymi.
4. Zmień hasło — plik `admin_start.txt` zostanie usunięty.
5. Wygeneruj konta uczniów/kuchni i przekaż hasła bezpiecznym kanałem.

Jeśli uruchomisz od razu usługę, dane znajdziesz w
`/opt/zsl_order/data/admin_start.txt` oraz w `journalctl -u zsl_order`.

## 6. Usługa systemd

`/etc/systemd/system/zsl_order.service`:

```ini
[Unit]
Description=ZSL Order - system stolowki szkolnej
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=zsl
Group=zsl
WorkingDirectory=/opt/zsl_order/data
ExecStart=/opt/zsl_order/zsl_order
Restart=on-failure
RestartSec=3

# Uprawnienia plików tworzonych przez aplikację (baza, admin_start.txt)
UMask=0077

# Hardening systemd
NoNewPrivileges=true
PrivateTmp=true
PrivateDevices=true
ProtectSystem=strict
ProtectHome=true
ProtectKernelTunables=true
ProtectKernelModules=true
ProtectControlGroups=true
RestrictAddressFamilies=AF_INET AF_INET6 AF_UNIX
RestrictNamespaces=true
LockPersonality=true
ReadWritePaths=/opt/zsl_order/data

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now zsl_order
sudo systemctl status zsl_order
sudo journalctl -u zsl_order -f
```

`ProtectSystem=strict` czyni system plików tylko do odczytu poza
`ReadWritePaths`; dlatego katalog `data/` musi być zapisywalny dla `zsl`.

## 7. Nginx + TLS

Przykład dla `zsl.example.edu`:

```nginx
# /etc/nginx/sites-available/zsl_order
server {
    listen 80;
    listen [::]:80;
    server_name zsl.example.edu;

    location /.well-known/acme-challenge/ {
        root /var/www/certbot;
    }

    location / {
        return 301 https://$host$request_uri;
    }
}

server {
    listen 443 ssl http2;
    listen [::]:443 ssl http2;
    server_name zsl.example.edu;

    ssl_certificate     /etc/letsencrypt/live/zsl.example.edu/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/zsl.example.edu/privkey.pem;
    ssl_protocols       TLSv1.2 TLSv1.3;
    ssl_prefer_server_ciphers off;

    # Aplikacja ma własny limit 64 KiB; to dodatkowa warstwa.
    client_max_body_size 64k;

    # Opcjonalny limit na logowanie (defense in depth).
    # limit_req_zone $binary_remote_addr zone=zsl_login:10m rate=10r/m;
    # location = /api/auth/login { limit_req zone=zsl_login burst=5 nodelay; proxy_pass ...; }

    location / {
        proxy_pass http://127.0.0.1:3000;
        proxy_http_version 1.1;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
        proxy_read_timeout 30s;
        proxy_send_timeout 30s;
    }
}
```

```bash
sudo ln -s /etc/nginx/sites-available/zsl_order /etc/nginx/sites-enabled/
sudo nginx -t && sudo systemctl reload nginx
```

Ważne:

- `Host $host` musi być zgodny z `Origin` przeglądarki — inaczej
  `origin_guard` odrzuci żądania zmieniające stan.
- Aplikacja **nie ufa** nagłówkom `X-Forwarded-*` do ustalania IP.
  Limiter logowania widzi adres proxy (`127.0.0.1`). Ograniczenie na
  konto nadal działa; limit per IP staje się limitem globalnym. Jeśli
  potrzebujesz limitowania per klient, uruchom aplikację bez proxy w
  zaufanej sieci albo dodaj obsługę zaufanego proxy w kodzie.
- Nie serwuj `frontend/` osobnym `root` w Nginx — aplikacja ma trasy
  `/login`, `/student`, `/kitchen`, `/admin` i fallback statyczny.
- HSTS jest dodawany przez aplikację przy `secure_cookies = true`; możesz
  dodatkowo ustawić go w Nginx.

Caddy (alternatywa):

```caddyfile
zsl.example.edu {
    encode zstd gzip
    reverse_proxy 127.0.0.1:3000 {
        header_up Host {host}
    }
}
```

## 8. Backup i odtwarzanie

### Backup

SQLite z WAL należy kopiować przez API backupu, nie przez `cp` w trakcie
zapisu:

```bash
sudo install -d -o zsl -g zsl -m 0700 /var/backups/zsl
sudo -u zsl sqlite3 /opt/zsl_order/data/baza.db \
  ".backup '/var/backups/zsl/baza-$(date +%F-%H%M).db'"
```

lub:

```bash
sudo -u zsl sqlite3 /opt/zsl_order/data/baza.db \
  "VACUUM INTO '/var/backups/zsl/baza-$(date +%F-%H%M).db'"
```

Cron (przykład, codziennie 2:15, retencja 30 dni):

```cron
15 2 * * * zsl sqlite3 /opt/zsl_order/data/baza.db ".backup '/var/backups/zsl/baza-$(date +\%F).db'" && find /var/backups/zsl -name 'baza-*.db' -mtime +30 -delete
```

Backup zawiera hash haseł i tokeny sesji — traktuj go jak dane wrażliwe
(min. `0600`, szyfrowanie, ograniczony dostęp).

### Odtwarzanie

```bash
sudo systemctl stop zsl_order
sudo -u zsl cp /var/backups/zsl/baza-2026-11-07.db /opt/zsl_order/data/baza.db
sudo -u zsl rm -f /opt/zsl_order/data/baza.db-wal /opt/zsl_order/data/baza.db-shm
sudo chmod 600 /opt/zsl_order/data/baza.db
sudo systemctl start zsl_order
```

Zawsze testuj odtworzenie na kopii i sprawdź logowanie oraz listę
zamówień. Backup bez testu odtworzenia nie jest backupem.

## 9. Aktualizacje

```bash
# 1. Backup
sudo -u zsl sqlite3 /opt/zsl_order/data/baza.db ".backup '/var/backups/zsl/przed-aktualizacja.db'"

# 2. Zatrzymanie
sudo systemctl stop zsl_order

# 3. Nowa binarka i frontend
sudo install -o root -g root -m 0755 target/release/zsl_order /opt/zsl_order/zsl_order
sudo rsync -a --delete frontend/ /opt/zsl_order/frontend/

# 4. Start i weryfikacja
sudo systemctl start zsl_order
sudo systemctl status zsl_order
curl -fsS https://zsl.example.edu/api/public/info >/dev/null && echo OK
```

Schemat jest idempotentny (`CREATE TABLE/INDEX IF NOT EXISTS`), więc
dodanie tabel/indeksów zwykle nie wymaga ręcznej migracji. Zmiany
usuwające/zmieniające kolumny wymagają **ręcznej migracji** — zawsze po
backupie i teście na kopii. Aplikacja nie ma frameworka migracji.

## 10. Monitoring

Minimum:

- proces i port: `systemctl status zsl_order`, `ss -ltnp | grep 3000`,
- health check: `GET /api/public/info` → 200,
- miejsce na dysku (baza + WAL + backupy),
- błędy w `journalctl -u zsl_order`,
- poprawność TLS (`openssl s_client`, certyfikaty),
- czas systemowy (NTP).

Przykładowy wpis w monitoringu HTTP:

```text
GET https://zsl.example.edu/api/public/info
oczekiwany status: 200
oczekiwany content-type: application/json
```

Aplikacja nie prowadzi access logów HTTP. Jeśli potrzebujesz logów
dostępowych, włącz je w Nginx, ale nie loguj ciasteczek, nagłówka
`X-CSRF-Token` ani ciał żądań. Ustaw retencję i dostęp do logów.

## 11. Testy i smoke test

Testy jednostkowe:

```bash
cargo test --locked
```

Test dymny na izolowanej kopii (nie na produkcji):

```bash
mkdir -p /tmp/zsl-test
cd /tmp/zsl-test
ln -sf /opt/zsl_order/frontend frontend
cat > config.json <<'JSON'
{
  "ip": "127.0.0.1",
  "port": 38080,
  "max_connections": 8,
  "db_url": "sqlite://baza.db",
  "session_hours": 12,
  "secure_cookies": false
}
JSON
/opt/zsl_order/zsl_order > server.log 2>&1 &
sleep 2
cat admin_start.txt

# Publiczny endpoint
curl -fsS http://127.0.0.1:38080/api/public/info | jq .

# Logowanie admina (podstaw dane z admin_start.txt)
curl -s -c cookies.txt -H 'Content-Type: application/json' \
  -d '{"login":"admin-...","password":"..."}' \
  http://127.0.0.1:38080/api/auth/login | jq .
```

Po testach zatrzymaj proces i usuń katalog testowy, aby produkcyjna baza
nie została przypadkowo użyta. Testy API można rozszerzyć o:

- wymuszoną zmianę hasła,
- wygenerowanie kont i logowanie ucznia,
- złożenie/anulowanie zamówienia,
- pobranie QR i pickup po kodzie oraz `ZSL:<token>`,
- przejście statusów do `completed` i sprawdzenie utargu,
- próbę dostępu ucznia do `/api/admin/*` (403),
- żądanie bez CSRF (403) i z obcym `Origin` (403),
- obecność nagłówków CSP/HSTS.

## 12. Rozwiązywanie problemów

| Objaw | Prawdopodobna przyczyna | Rozwiązanie |
| --- | --- | --- |
| `Nie można użyć 127.0.0.1:3000` i start na `0.0.0.0:3000` | port zajęty | zwolnij port lub zmień `port`; sprawdź `ss -ltnp` |
| brak ciasteczka po logowaniu | `secure_cookies=true` bez HTTPS | włącz HTTPS lub ustaw `false` na czas testu |
| 403 przy POST | brak/nieprawidłowy `X-CSRF-Token` lub obcy `Origin` | pobierz token z `/api/auth/me`, ustaw poprawny `Host`/`Origin` |
| 403 `password_change_required` | konto ma hasło startowe | zmień hasło przez `/api/auth/change-password` |
| 401 po restarcie | sesja wygasła lub użytkownik dezaktywowany | zaloguj ponownie |
| brak stron/404 na CSS/JS | brak `frontend/` w katalogu roboczym | symlink lub kopia `frontend/` |
| `database is locked` | zbyt wiele zapisów / inne połączenie | zwiększ `busy_timeout` (w kodzie 5 s), ogranicz `max_connections`, sprawdź inne procesy |
| brak danych startowego admina | admin już istnieje | użyj istniejącego konta lub procedury awaryjnej przez DB |
| brak admina aktywnego | wszyscy admini dezaktywowani | przywróć `active=1` w DB lub utwórz konto poza aplikacją (z backupem) |

## 13. Procedura awaryjna

- **Zgubione hasło admina:** zatrzymaj usługę, zrób backup, w bazie ustaw
  `must_change_password=1` i tymczasowy hash wygenerowany poza aplikacją
  albo usuń konto admina tylko wtedy, gdy nie ma innych adminów (wtedy
  bootstrap utworzy nowe przy starcie — ale to operacja ryzykowna).
  Bezpieczniej: przywróć dostęp przez administratora systemu i backup.
- **Podejrzenie przejęcia konta:** dezaktywuj konto (unieważnia sesje),
  zmień hasło/reset, przejrzyj audyt, sprawdź logi proxy, rozważ
  unieważnienie wszystkich sesji (usunięcie wpisów `sessions`).
- **Kompromitacja bazy:** unieważnij wszystkie sesje (`DELETE FROM
  sessions`), wymuś zmianę haseł, przeanalizuj kopie/backupy, zgłoś
  incydent zgodnie z procedurą szkoły.
- **Awaria dysku:** odtwórz binarkę, `frontend/`, `config.json` i ostatni
  backup bazy; sprawdź uprawnienia; przetestuj logowanie i zamówienie.

## 14. Skalowanie i ograniczenia wdrożenia

- Jedna instancja aplikacji i jedna baza SQLite. Nie uruchamiaj wielu
  procesów na tym samym pliku przez NFS.
- SQLite WAL dobrze obsługuje wielu czytelników i jednego pisarza;
  dla szkoły wystarcza.
- Brak replikacji i klastra — backup i szybkie odtworzenie są strategią
  ciągłości działania.
- Przy wzroście wymagań rozważ PostgreSQL i osobną warstwę aplikacji, ale
  to zmiana architektury, nie konfiguracji.
