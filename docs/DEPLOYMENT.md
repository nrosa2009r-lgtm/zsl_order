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
    ├── .env               # 0600, tryb wdrożenia i PUBLIC_URL
    ├── baza.db            # 0600
    ├── baza.db-wal        # tworzone przez SQLite
    ├── baza.db-shm
    ├── jwt.key            # sekret HMAC JWT, 0600, tworzony przy starcie
    ├── logs/              # dzienne logi, 0700, tworzone przy starcie
    │   └── zsl_order.log.RRRR-MM-DD
    ├── backups/           # kopie bazy, 0700 i 0600, tworzone przez `backup`
    └── admin_start.txt    # tylko przy pierwszym uruchomieniu, 0600
```

Aplikacja używa **ścieżek względnych** (`config.json`, `.env`, `baza.db`,
`jwt.key`, `logs/`, `admin_start.txt`, `frontend/`) wobec katalogu
roboczego. Dlatego `WorkingDirectory` musi wskazywać `data/`, a
`frontend/` musi być dostępny względem tego katalogu. Można użyć
symlinku:

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

Przy pierwszym starcie aplikacja tworzy w `data/`:

- `baza.db` i pliki WAL,
- `jwt.key` (sekret HMAC do tokenów JWT, `0600`),
- `logs/` (`0700`) z dziennymi plikami logów,
- `admin_start.txt` (`0600`) tylko raz, gdy nie ma żadnego admina.

`sudo install -d` i `UMask=0077` w systemd zapewniają, że nowe pliki nie
są czytelne dla innych użytkowników.

## 4. Konfiguracja

Konfiguracja jest podzielona na dwa pliki:

- `.env` — tryb wdrożenia i granica zaufania (wzór `.env.example`),
- `config.json` — baza, pula połączeń i sesje (wzór
  `config.example.json`).

`.env` jest ignorowany przez Git i nie zawiera sekretów. Gdy ta sama
wartość występuje w obu plikach, `.env` ma pierwszeństwo.

### 4.1. Uruchomienie lokalne

Wariant dla sieci szkolnej, bez tunelu i bez publicznego adresu.

```bash
cd /opt/zsl_order/data
sudo -u zsl cp /opt/zsl_order/config.example.json config.json
sudo -u zsl cp /opt/zsl_order/.env.example .env
sudo chmod 600 config.json .env
sudo -u zsl editor .env
```

Przykładowy `.env`:

```dotenv
DEPLOY_MODE=local
BIND_ADDR=0.0.0.0:8080
TRUSTED_PROXIES=127.0.0.1,::1
PUBLIC_URL=http://192.168.1.10:8080
COOKIE_SECURE=false
```

- `BIND_ADDR` musi być adresem, pod którym uczniowie mają łączyć się z
  serwerem, np. `0.0.0.0:8080` oznacza wszystkie interfejsy.
- `PUBLIC_URL` jest opcjonalny. Jeśli go podasz, wpisz dokładny adres
  używany w przeglądarce (schemat, host i port).
- `COOKIE_SECURE=auto` ustawia `Secure`, gdy `PUBLIC_URL` używa HTTPS.
  Przy czystym HTTP w sieci lokalnej użyj `false`.
- `CF-Connecting-IP` jest w tym trybie całkowicie ignorowany, nawet jeśli
  ktoś go podrobi.
- Otwórz w firewallu wyłącznie port z `BIND_ADDR`, np. `8080/tcp`.

Po zmianie `.env` zrestartuj usługę:

```bash
sudo systemctl restart zsl_order
```

### 4.2. Dołączenie Cloudflare Tunnel

Wariant publiczny: cloudflared łączy się z Cloudflare, a aplikacja
nasłuchuje wyłącznie na loopbacku. Cloudflare kończy TLS, więc aplikacja
widzi zwykłe HTTP.

W panelu Cloudflare utwórz tunel i rekord DNS dla wybranej domeny,
np. `zsl.example.com`. Konfigurację Access, WAF i rate limitingu panelu
Cloudflare opisujemy tylko jako zalecenie — aplikacja jej nie zastępuje.

Przykładowy `/etc/cloudflared/config.yml`:

```yaml
tunnel: REPLACE_TUNNEL_ID
credentials-file: /etc/cloudflared/REPLACE_TUNNEL_ID.json

ingress:
  - hostname: zsl.example.com
    service: http://127.0.0.1:3000
  - service: http_status:404
```

Przykładowa usługa `/etc/systemd/system/cloudflared.service`:

```ini
[Unit]
Description=Cloudflare Tunnel dla ZSL Order
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=cloudflared
Group=cloudflared
ExecStart=/usr/bin/cloudflared --no-autoupdate tunnel run
Restart=on-failure
RestartSec=5
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ProtectHome=true

[Install]
WantedBy=multi-user.target
```

Przykładowy `.env` aplikacji:

```dotenv
DEPLOY_MODE=tunnel
BIND_ADDR=127.0.0.1:3000
TRUSTED_PROXIES=127.0.0.1,::1
PUBLIC_URL=https://zsl.example.com
COOKIE_SECURE=auto
```

W tym trybie:

- `BIND_ADDR` musi wskazywać `127.0.0.1` lub `::1`; aplikacja odmówi
  startu dla `0.0.0.0`.
- `PUBLIC_URL` jest wymagany. `Host` i `Origin` muszą wskazywać ten
  adres; wyjątkiem są lokalne adresy pętlowe używane do diagnostyki.
- `CF-Connecting-IP` jest honorowany tylko wtedy, gdy połączenie przyszło
  z adresu należącego do `TRUSTED_PROXIES`.
- `COOKIE_SECURE` jest zawsze wymuszane na `true`, niezależnie od wartości
  w `.env`, bo przeglądarka łączy się z Cloudflare po HTTPS.
- HSTS i `upgrade-insecure-requests` są dodawane tylko w tym trybie.
- Nie otwieraj portu aplikacji w firewallu. Jedynym klientem ma być
  cloudflared na tym samym hoście.

Usługę aplikacji uruchom według rozdziału 6. Cloudflared:

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now cloudflared
sudo systemctl status cloudflared
```

### 4.3. Przełączanie trybów

Przełączenie polega wyłącznie na zmianie `.env` i restarcie usługi — bez
przebudowy binarki i bez zmian w bazie:

```bash
sudo -u zsl editor /opt/zsl_order/data/.env
sudo systemctl restart zsl_order
```

Po zmianie trybu użytkownicy mogą zostać wylogowani, jeśli zmieni się
flaga `Secure` ciasteczka lub adres publiczny. Nie trzeba czyścić bazy.

Czas życia tokenów JWT jest stały (**15 minut**) i nie jest konfigurowany
w `config.json`. Sekret `jwt.key` jest generowany automatycznie; nie ma
osobnego pola konfiguracyjnego.

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
EnvironmentFile=/opt/zsl_order/data/.env
ExecStart=/opt/zsl_order/zsl_order
Restart=on-failure
RestartSec=3

# Uprawnienia plików tworzonych przez aplikację
# (baza, jwt.key, logs/, admin_start.txt)
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

Aplikacja rotuje logi dziennie (`logs/zsl_order.log.RRRR-MM-DD`), ale nie
usuwa starych plików. Dodaj `logrotate` (albo `tmpfiles.d`), np.
`/etc/logrotate.d/zsl_order`:

```text
/opt/zsl_order/data/logs/zsl_order.log.* {
    daily
    rotate 30
    missingok
    notifempty
    compress
    delaycompress
    copytruncate
    su zsl zsl
}
```

Katalog `logs/` powinien mieć `0700`; pliki nie zawierają haseł ani
tokenów, ale mogą ujawniać strukturę ruchu i identyfikatory techniczne.

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

### 8.1. Wbudowana kopia zapasowa

SQLite w trybie WAL należy kopiować przez API backupu, nie przez `cp`
przy działającej aplikacji. Binarka ma do tego polecenie `backup`, które:

1. wykonuje `VACUUM INTO` do pliku tymczasowego w `backup_dir`,
2. sprawdza kopię przez `PRAGMA integrity_check`,
3. ustawia uprawnienia `0600` i atomowo zmienia nazwę na docelową,
4. usuwa najstarsze kopie zgodnie z retencją, ale **zawsze zachowuje
   co najmniej 7 najnowszych**.

Kopia ma nazwę `zsl-order-RRRRMMDD-HHMMSS.db` (czas `Europe/Warsaw`).
Nie zawiera plików `-wal` ani `-shm`; jest spójnym, samodzielnym plikiem
SQLite.

Ręczne wykonanie (katalogiem roboczym musi być `data/`):

```bash
sudo -u zsl sh -c 'cd /opt/zsl_order/data && /opt/zsl_order/zsl_order backup'
sudo -u zsl sh -c 'cd /opt/zsl_order/data && /opt/zsl_order/zsl_order backup --keep-days 14'
sudo -u zsl sh -c 'cd /opt/zsl_order/data && /opt/zsl_order/zsl_order backup --verify backups/zsl-order-20261111-031500.db'
```

`--keep-days` nadpisuje konfigurację tylko dla tego uruchomienia.
`--verify` otwiera istniejącą kopię tylko do odczytu i uruchamia
`PRAGMA integrity_check`; nie tworzy ani nie zmienia plików. Pełna pomoc:
`zsl_order backup --help`.

Konfiguracja (`.env` ma pierwszeństwo nad `config.json`):

| `config.json` | `.env` | Domyślnie | Znaczenie |
| --- | --- | --- | --- |
| `backup_dir` | `BACKUP_DIR` | `backups` | katalog kopii, względny wobec `data/` lub absolutny |
| `backup_keep_days` | `BACKUP_KEEP_DAYS` | `30` | wiek kopii w dniach; `0` oznacza „zostaw tylko 7 najnowszych” |

Retencja opiera się na czasie modyfikacji pliku. Kopie spoza wzorca
`zsl-order-*.db` nie są usuwane. Katalog `backups/` jest tworzony z
trybem `0700`, a każda kopia ma `0600`.

### 8.2. Codzienny timer systemd

Jednostki są w repozytorium w `deploy/systemd/`. Na serwerze skopiuj je
do `/etc/systemd/system/` albo utwórz ręcznie:

`/etc/systemd/system/zsl-order-backup.service`:

```ini
[Unit]
Description=ZSL Order - kopia zapasowa bazy danych
Documentation=file:/opt/zsl_order/docs/DEPLOYMENT.md
After=zsl-order.service
Wants=zsl-order.service
OnFailure=zsl-order-backup-failed.service

[Service]
Type=oneshot
User=zsl
Group=zsl
WorkingDirectory=/opt/zsl_order/data
EnvironmentFile=-/opt/zsl_order/data/.env
ExecStart=/opt/zsl_order/zsl_order backup
Nice=10
IOSchedulingClass=best-effort
IOSchedulingPriority=7
UMask=0077
NoNewPrivileges=true
PrivateTmp=true
PrivateDevices=true
ProtectSystem=strict
ProtectHome=true
ProtectKernelTunables=true
ProtectKernelModules=true
ProtectControlGroups=true
RestrictAddressFamilies=AF_UNIX
RestrictNamespaces=true
LockPersonality=true
SystemCallArchitectures=native
ReadWritePaths=/opt/zsl_order/data
```

`/etc/systemd/system/zsl-order-backup.timer`:

```ini
[Unit]
Description=ZSL Order - codzienna kopia zapasowa

[Timer]
OnCalendar=*-*-* 03:15:00
Persistent=true
AccuracySec=1min
Unit=zsl-order-backup.service

[Install]
WantedBy=timers.target
```

`/etc/systemd/system/zsl-order-backup-failed.service`:

```ini
[Unit]
Description=ZSL Order - powiadomienie o nieudanej kopii zapasowej

[Service]
Type=oneshot
ExecStart=/usr/bin/logger -t zsl-order-backup -p daemon.err "Kopia zapasowa ZSL Order nie powiodla sie. Sprawdz: journalctl -u zsl-order-backup.service"
```

`OnFailure=` uruchamia jednostkę powiadamiającą, gdy kopia się nie uda.
Domyślnie wpis trafia do journald z priorytetem `err`. W środowisku
produkcyjnym podłącz tam własny kanał (e-mail, monitoring, webhook) —
samo `logger` nie zastępuje alertu poza serwerem.

Włączenie i test:

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now zsl-order-backup.timer
sudo systemctl list-timers zsl-order-backup.timer
sudo systemctl start zsl-order-backup.service
sudo journalctl -u zsl-order-backup.service -n 50 --no-pager
```

`Persistent=true` uruchamia zaległą kopię po restarcie hosta. Jeśli
godzina 03:15 koliduje z innymi zadaniami, zmień `OnCalendar` i wykonaj
`systemctl daemon-reload`. Kopia działa niezależnie od tego, czy
aplikacja jest uruchomiona; `VACUUM INTO` jest bezpieczne dla WAL.

### 8.3. Kopiowanie poza serwer

Kopia na tym samym dysku nie chroni przed awarią hosta ani ransomware.
Po każdym backupie (albo osobnym timerem) skopiuj katalog na drugie
urządzenie przez `rsync` po SSH:

```bash
# Jednorazowo: klucz SSH dla użytkownika zsl i katalog docelowy.
sudo -u zsl ssh-keygen -t ed25519 -N '' -f /opt/zsl_order/data/.ssh-backup
sudo -u zsl ssh-copy-id -i /opt/zsl_order/data/.ssh-backup.pub backup@backup-host

sudo -u zsl rsync -a --delete \
  -e 'ssh -i /opt/zsl_order/data/.ssh-backup -o IdentitiesOnly=yes' \
  /opt/zsl_order/data/backups/ backup@backup-host:/srv/backup/zsl/
```

`--delete` powoduje, że zdalny katalog jest lustrem lokalnego — po
lokalnej retencji stare kopie znikną też zdalnie. Jeśli chcesz trzymać
dłuższą historię poza serwerem, usuń `--delete` i ustaw osobną retencję
na urządzeniu docelowym.

Przykładowy cron użytkownika `zsl` (kopiowanie o 03:45, po backupie):

```cron
45 3 * * * rsync -a --delete -e 'ssh -i /opt/zsl_order/data/.ssh-backup -o IdentitiesOnly=yes' /opt/zsl_order/data/backups/ backup@backup-host:/srv/backup/zsl/ >> /opt/zsl_order/data/rsync-backup.log 2>&1
```

Zadbaj, aby drugie urządzenie miało szyfrowany dysk, ograniczony dostęp
i własną kopię zapasową. Kopia zdalna zawiera te same dane wrażliwe co
lokalna.

### 8.4. Szyfrowanie poza aplikacją (`age`)

Aplikacja nie szyfruje kopii — robi to administrator narzędziem `age`
albo innym systemem szyfrowania. Klucz publiczny może być na serwerze,
prywatny trzymaj **poza serwerem** (np. w sejfie lub menedżerze haseł).

```bash
# Jednorazowo, poza serwerem produkcyjnym:
age-keygen -o zsl-backup-age.key
# age1... to klucz publiczny; wpisz go do skryptu na serwerze.

# Na serwerze, po utworzeniu kopii:
age -r age1... -o /var/backups/zsl-out/zsl-order-20261111-031500.db.age \
  /opt/zsl_order/data/backups/zsl-order-20261111-031500.db

# Odszyfrowanie na potrzeby testu odtworzenia (na osobnym hoście):
age -d -i zsl-backup-age.key -o kopia.db kopia.db.age
/opt/zsl_order/zsl_order backup --verify kopia.db
```

Szyfruj zwłaszcza kopie opuszczające serwer. Nie umieszczaj klucza
prywatnego obok zaszyfrowanych kopii — wtedy szyfrowanie nie chroni
przed przejęciem hosta.

### 8.5. Odtwarzanie (tylko ręczne)

Aplikacja **nie ma** polecenia `restore`. Odtwarzanie jest celowo
procedurą ręczną, wykonywaną przy zatrzymanej usłudze:

1. Zatrzymaj usługę i zweryfikuj wybraną kopię:

   ```bash
   sudo systemctl stop zsl_order
   sudo -u zsl sh -c 'cd /opt/zsl_order/data && /opt/zsl_order/zsl_order backup --verify backups/zsl-order-20261111-031500.db'
   ```

2. Odsuń obecną bazę i pliki WAL (nie usuwaj ich od razu — mogą być
   potrzebne do analizy):

   ```bash
   cd /opt/zsl_order/data
   sudo -u zsl mkdir -p restore-old
   sudo -u zsl mv baza.db "restore-old/baza.db.$(date +%F-%H%M%S)"
   for plik in baza.db-wal baza.db-shm; do
     [ -e "$plik" ] && sudo -u zsl mv "$plik" "restore-old/$plik.$(date +%F-%H%M%S)"
   done
   ```

3. Wstaw kopię i ustaw uprawnienia:

   ```bash
   sudo -u zsl cp backups/zsl-order-20261111-031500.db baza.db
   sudo -u zsl chmod 600 baza.db
   ```

4. Uruchom aplikację i sprawdź integralność oraz działanie:

   ```bash
   sudo systemctl start zsl_order
   sudo -u zsl sh -c 'cd /opt/zsl_order/data && /opt/zsl_order/zsl_order backup --verify baza.db'
   sudo journalctl -u zsl_order -n 50 --no-pager
   curl -fsS http://127.0.0.1:3000/api/healthz
   ```

5. Zaloguj się do panelu admina, sprawdź listę zamówień i raport
   dzienny. Dopiero po potwierdzeniu poprawności usuń katalog
   `restore-old/`.

Przywrócenie starszej kopii i uruchomienie nowszej binarki jest
bezpieczne: `sqlx` doda brakujące migracje przy starcie. Jeśli kopia
pochodzi z wersji sprzed migracji, zachowaj `restore-old/` do czasu
potwierdzenia danych.

Sekret `jwt.key` **nie musi** być backupowany: tokeny JWT żyją 15 minut, a
po odtworzeniu bazy ze starym `jwt.key` i tak wygasną. Jeśli chcesz
zachować ważność tokenów przez odtworzenie, zrób kopię `jwt.key` razem z
bazą — ale wtedy traktuj ją jak klucz kryptograficzny. Brak `jwt.key`
spowoduje wygenerowanie nowego sekretu przy starcie i unieważni
wszystkie tokeny (sesje ciasteczkowe nadal działają).

### 8.6. Checklist backupu

- [ ] `backup_dir` wskazuje katalog na dysku z wystarczającą ilością
      miejsca; katalog ma `0700`, kopie `0600`.
- [ ] `zsl-order-backup.timer` jest włączony i widoczny w
      `systemctl list-timers`.
- [ ] Ręczne `systemctl start zsl-order-backup.service` kończy się
      sukcesem, a `journalctl` nie zawiera błędów.
- [ ] `zsl_order backup --verify` przechodzi dla najnowszej kopii.
- [ ] Retencja zachowuje co najmniej 7 najnowszych kopii.
- [ ] Kopie są regularnie wysyłane poza serwer (`rsync`) i zdalny katalog
      jest sprawdzany.
- [ ] Kopie opuszczające serwer są szyfrowane (`age`), a klucz prywatny
      jest przechowywany poza serwerem.
- [ ] Odtworzenie zostało przetestowane na izolowanej kopii, wraz z
      logowaniem i sprawdzeniem zamówień.
- [ ] `OnFailure=` prowadzi do kanału, który faktycznie czyta człowiek.

Backup bez testu odtworzenia nie jest backupem.


## 9. Aktualizacje

```bash
# 1. Backup
sudo systemctl start zsl-order-backup.service
sudo journalctl -u zsl-order-backup.service -n 20 --no-pager

# 2. Zatrzymanie
sudo systemctl stop zsl_order

# 3. Nowa binarka i frontend
sudo install -o root -g root -m 0755 target/release/zsl_order /opt/zsl_order/zsl_order
sudo rsync -a --delete frontend/ /opt/zsl_order/frontend/

# 4. Start i weryfikacja
sudo systemctl start zsl_order
sudo systemctl status zsl_order
curl -fsS https://zsl.example.edu/api/healthz >/dev/null && echo OK
```

Schemat jest zarządzany przez **wbudowane migracje `sqlx`** z katalogu
`migrations/`. Nowa binarka sama wykonuje brakujące migracje przy starcie
i zapisuje wersje w tabeli `_sqlx_migrations`. Migracja bazowa używa
`CREATE TABLE/INDEX IF NOT EXISTS`, więc działa też na bazie utworzonej
przez starszą wersję aplikacji (bez `_sqlx_migrations`) i nie usuwa
danych.

**Przed każdą aktualizacją zrób kopię pliku bazy** (obok zwykłego backupu):

```bash
sudo systemctl start zsl-order-backup.service
sudo -u zsl sh -c 'cd /opt/zsl_order/data && /opt/zsl_order/zsl_order backup --verify $(ls -1t backups/zsl-order-*.db | head -n1)'
```

Kopię wykonuj przy zatrzymanej aplikacji albo przez wbudowane `backup`
(bezpieczne dla WAL). Następnie uruchom nową binarkę i sprawdź
`_sqlx_migrations` oraz `PRAGMA integrity_check`. Migracje są addytywne;
zmiany usuwające kolumny nadal wymagają ręcznej procedury i testu na
kopii.

Aktualizacja **nie usuwa** `jwt.key`, `logs/` ani bazy. Nowa binarka
może wymagać restartu, aby wczytać nowe trasy; nie usuwaj `jwt.key`,
jeśli chcesz zachować ważność wydanych tokenów (i tak wygasają po 15
minutach).

## 10. Monitoring

Minimum:

- proces i port: `systemctl status zsl_order`, `ss -ltnp | grep 3000`,
- health check: `GET /api/healthz` → 200 (sprawdza też bazę),
- miejsce na dysku (baza + WAL + `logs/` + `backups/`),
- błędy w `journalctl -u zsl_order`,
- `systemctl list-timers zsl-order-backup.timer` oraz
  `journalctl -u zsl-order-backup.service` (ostatnia kopia, błędy),
- dzienne logi w `/opt/zsl_order/data/logs/` (rotacja i retencja),
- poprawność TLS (`openssl s_client`, certyfikaty),
- czas systemowy (NTP).

Przykładowy wpis w monitoringu HTTP:

```text
GET https://zsl.example.edu/api/healthz
oczekiwany status: 200
oczekiwany content-type: application/json
```

Aplikacja prowadzi własne logi techniczne (`logs/zsl_order.log.*`) z
metodą, ścieżką, statusem i czasem odpowiedzi — **bez** ciasteczek,
nagłówka `Authorization`/`X-CSRF-Token`, ciał żądań i adresów IP. Jeśli
potrzebujesz logów dostępowych z IP, włącz je w Nginx, ale nie loguj
ciastek, tokenów ani ciał żądań. Ustaw retencję i dostęp do logów.

## 11. Testy i smoke test

> **Uwaga:** dane logowania używane w skryptach testowych są **wyłącznie
> testowe**. Testy wykonuj na izolowanej kopii bazy, nigdy na prawdziwej
> instancji, i nie używaj testowych danych logowania w produkcji.

Testy jednostkowe:

```bash
cargo test --locked
```

Skrypty smoke (izolowane bazy w `/tmp/opencode`, dane wyłącznie testowe):

```bash
cargo build
scripts/smoke-modes.sh       # tryby local/tunnel i nagłówki
scripts/smoke-hardening.sh   # hasła, import CSV, RBAC, limity, sesje
scripts/test-migrations.sh   # pusta baza + baza legacy
scripts/smoke-orders.sh      # idempotencja, wyścigi, płatności, raport/CSV
scripts/smoke-backup.sh      # kopia WAL, integralność, retencja min. 7, --verify
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
curl -fsS http://127.0.0.1:38080/api/healthz | jq .

# Logowanie admina (podstaw dane z admin_start.txt)
curl -s -c cookies.txt -H 'Content-Type: application/json' \
  -d '{"login":"admin-...","password":"..."}' \
  http://127.0.0.1:38080/api/auth/login | jq .

# Wariant API: token JWT (bez ciasteczka i bez CSRF)
TOKEN=$(curl -s -H 'Content-Type: application/json' \
  -d '{"login":"admin-...","password":"..."}' \
  http://127.0.0.1:38080/api/auth/token | jq -r '.access_token')
curl -fsS -H "Authorization: Bearer $TOKEN" \
  http://127.0.0.1:38080/api/auth/me | jq .
curl -s -X POST -H "Authorization: Bearer $TOKEN" \
  http://127.0.0.1:38080/api/auth/logout | jq .
# token po logout powinien zwrócić 401
curl -s -o /dev/null -w '%{http_code}\n' \
  -H "Authorization: Bearer $TOKEN" http://127.0.0.1:38080/api/auth/me

# Uprawnienia plików i logi
stat -c '%a %n' jwt.key logs admin_start.txt 2>/dev/null
ls -l logs/
```

Po testach zatrzymaj proces i usuń katalog testowy, aby produkcyjna baza
nie została przypadkowo użyta. Testy API można rozszerzyć o:

- wymuszoną zmianę hasła,
- wygenerowanie kont i logowanie ucznia,
- złożenie/anulowanie zamówienia,
- powtórzenie żądania z tym samym `idempotency_key` (bez duplikatu),
- dwa równoczesne anulowania (jedno `200`, jedno `409`),
- próbę ustawienia przez ucznia `status`/`price_cents`/`total_cents`,
- zmianę zamówienia z innego dnia przez kuchnię (`403`),
- przejście statusów do `completed`, `paid_at`/`paid_by` i sprawdzenie
  raportu dziennego oraz CSV (gotówka/karta),
- automatyczne oznaczenie nieodebranego i blokadę po `max_uncollected`,
- pobranie QR i pickup po kodzie oraz `ZSL:<token>`,
- próbę dostępu ucznia do `/api/admin/*` (403),
- żądanie bez CSRF (403) i z obcym `Origin` (403),
- wydanie tokenu JWT, żądanie Bearer bez CSRF, logout i ponowne żądanie
  (401) oraz odrzucenie tokenu po zmianie hasła/dezaktywacji,
- obecność nagłówków CSP/HSTS i brak danych wrażliwych w logach.

## 12. Rozwiązywanie problemów

| Objaw | Prawdopodobna przyczyna | Rozwiązanie |
| --- | --- | --- |
| `Nie można użyć 127.0.0.1:3000` i start na `0.0.0.0:3000` | port zajęty | zwolnij port lub zmień `port`; sprawdź `ss -ltnp` |
| brak ciasteczka po logowaniu | `secure_cookies=true` bez HTTPS | włącz HTTPS lub ustaw `false` na czas testu |
| 403 przy POST | brak/nieprawidłowy `X-CSRF-Token` lub obcy `Origin` | pobierz token z `/api/auth/me`, ustaw poprawny `Host`/`Origin` |
| 403 `password_change_required` | konto ma hasło startowe | zmień hasło przez `/api/auth/change-password` |
| 401 po restarcie | sesja wygasła lub użytkownik dezaktywowany | zaloguj ponownie |
| 401 na tokenie JWT | token wygasł (15 min), został unieważniony przez logout/zmianę hasła/dezaktywację, usunięto `jwt.key` lub baza nie ma wiersza sesji JWT | pobierz nowy token przez `/api/auth/token`; nie próbuj „naprawiać” tokenu |
| `jwt.key` należy do roota lub usługa nie może go odczytać | plik utworzony przez innego użytkownika | `chown zsl:zsl jwt.key && chmod 600 jwt.key`, restart (aplikacja sama wymusza tryb 0600, ale nie właściciela) |
| brak katalogu `logs/` lub brak zapisu | `ProtectSystem`/właściciel katalogu | upewnij się, że `data/` jest w `ReadWritePaths` i należy do `zsl`; katalog `logs/` tworzy aplikacja |
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
- **Podejrzenie przejęcia konta:** dezaktywuj konto (unieważnia sesje
  cookie i JWT), zmień hasło/reset, przejrzyj audyt, sprawdź logi proxy,
  rozważ unieważnienie wszystkich sesji (usunięcie wpisów `sessions`).
- **Wyciek tokenu JWT:** usuń powiązany wiersz z `sessions` (albo
  wszystkie wpisy JWT: `token_hash` zaczynające się od skrótu
  `sha256("jwt:...")`; praktycznie `DELETE FROM sessions`), zmień hasło
  konta. Token i tak wygaśnie po 15 minutach.
- **Podejrzenie wycieku `jwt.key`:** zatrzymaj usługę, usuń `jwt.key`,
  uruchom ponownie (nowy sekret), co unieważnia wszystkie tokeny JWT;
  przeanalizuj dostęp do hosta i logi. Sesje ciasteczkowe pozostają
  ważne — w razie potrzeby usuń je z `sessions`.
- **Kompromitacja bazy:** unieważnij wszystkie sesje (`DELETE FROM
  sessions`), wymuś zmianę haseł, przeanalizuj kopie/backupy, zgłoś
  incydent zgodnie z procedurą szkoły.
- **Awaria dysku:** odtwórz binarkę, `frontend/`, `config.json` i ostatni
  backup bazy; sprawdź uprawnienia; przetestuj logowanie i zamówienie.
  `jwt.key` możesz odtworzyć (zostanie wygenerowany) — spowoduje to
  unieważnienie tokenów.

## 14. Skalowanie i ograniczenia wdrożenia

- Jedna instancja aplikacji i jedna baza SQLite. Nie uruchamiaj wielu
  procesów na tym samym pliku przez NFS.
- SQLite WAL dobrze obsługuje wielu czytelników i jednego pisarza;
  dla szkoły wystarcza.
- Brak replikacji i klastra — backup i szybkie odtworzenie są strategią
  ciągłości działania.
- Przy wzroście wymagań rozważ PostgreSQL i osobną warstwę aplikacji, ale
  to zmiana architektury, nie konfiguracji.
