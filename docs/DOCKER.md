# ZSL Order w Dockerze

Obraz jest dwuetapowy: w pierwszym etapie kod kompiluje Rust 1.98, w drugim
zostaje minimalny Debian z samą binarką, frontendem i `curl` (tylko do
healthchecku). Rozmiar obrazu to około 150 MB.

Aplikacja w kontenerze działa tak samo jak natywnie: nasłuchuje na
`0.0.0.0:3000` (wewnątrz kontenera), zapisuje dane w katalogu roboczym `/data`, a `frontend/`
jest dowiązaniem do `/app/frontend`. Katalog `/data` jest wolumenem, więc
baza, klucz JWT i kopie zapasowe przeżywają restart i aktualizację obrazu.

## 1. Wymagania

- Docker Engine 24+ z Compose v2 (`docker compose version`)
- wolne porty **11000** (HTTP) i **10000** (HTTPS) na hoście
- 1 GB RAM i około 1,5 GB miejsca na obraz i warstwy budowania

Jeśli aplikacja działa też natywnie przez systemd, zatrzymaj ją przed
uruchomieniem kontenera — inaczej port 11000 będzie zajęty:

```bash
sudo systemctl stop zsl-order
```

## 2. Start w 3 poleceniach (HTTP na porcie 11000)

Z katalogu z repozytorium:

```bash
docker compose up -d --build
docker compose ps
docker compose logs -f
```

Po chwili status to `Up (healthy)`. Adres w sieci lokalnej:
`http://<ADRES-SYSTEMU>:11000`, gdzie `<ADRES-SYSTEMU>` to adres IP
komputera albo jego nazwa w sieci (np. `zsl-serwer.local`).

Odbierz dane administratora (tylko przy pierwszym starcie, na świeżym
wolumenie):

```bash
docker compose exec zsl-order cat /data/admin_start.txt
```

Po pierwszym zalogowaniu i zmianie hasła plik znika. Aby wygenerować nowe
konto startowe, trzeba usunąć wolumen (patrz §9).

> Wpisuj adres z jawnym `http://`. Przeglądarki domyślnie próbują HTTPS
> (HTTPS-First), a ten serwer nie mówi po TLS — szczegóły w §4.

## 3. HTTPS w sieci lokalnej (Caddy, port 10000)

Do skanowania kodów QR kamera wymaga bezpiecznego kontekstu: HTTPS z
zaufanym certyfikatem albo `localhost`. W sieci lokalnej najprościej
postawić Caddy jako reverse proxy z własnym CA.

```bash
# SITE_HOST = adres IP komputera w sieci lokalnej (bez http://)
SITE_HOST=<ADRES-SYSTEMU> docker compose \
  -f docker-compose.yml -f docker-compose.https.yml up -d
```

Aby ustawienie było trwałe, zapisz je w pliku `.env` w katalogu projektu
(Compose czyta go automatycznie):

```dotenv
SITE_HOST=<ADRES-SYSTEMU>
```

Wtedy wystarczy:

```bash
docker compose -f docker-compose.yml -f docker-compose.https.yml up -d
```

Efekt: `https://<ADRES-SYSTEMU>:10000`, a aplikacja pozostaje dostępna po
HTTP na porcie 11000 (`http://<ADRES-SYSTEMU>:11000`). Porty 80, 443 i 3000
na hoście nie są używane, więc nie kolidują z innymi usługami.

### Adres klienta za Caddy (limity per IP)

Caddy stoi przed aplikacją, więc bez przekazania adresu klienta aplikacja
widzi tylko adres kontenera Caddy. Wszyscy użytkownicy trafiliby wtedy do
jednego koszyka limitu prób logowania — jedna osoba mogłaby zablokować
logowanie całej szkole.

Nakładka HTTPS ustawia dlatego:

- `CLIENT_IP_HEADER=X-Forwarded-For` — aplikacja czyta adres klienta z
  tego nagłówka,
- `TRUSTED_PROXIES=172.28.0.2/32` — tylko ten adres (Caddy w dedykowanej
  sieci `zsl-proxy`) jest zaufany.

Caddy **nadpisuje** `X-Forwarded-For` adresem rzeczywistego klienta
(`header_up X-Forwarded-For {remote_host}`), więc klient nie może
podmienić adresu własnym nagłówkiem. Połączenie bezpośrednie z portem
11000 przychodzi z innego adresu (bramy sieci Compose), nie jest zaufane
i podrobiony nagłówek jest ignorowany.

Gdy nagłówka brakuje lub jest niepoprawny, aplikacja loguje ostrzeżenie
(maks. raz na minutę) i wraca do adresu proxy. Wtedy limity per IP znów są
wspólne — sprawdź wtedy konfigurację Caddy i `TRUSTED_PROXIES`.

Jeśli podsieć `172.28.0.0/24` koliduje z inną siecią na hoście, zmień
`subnet` w `docker-compose.https.yml` **oraz** adres w `TRUSTED_PROXIES`
i `ipv4_address` Caddy.

### Zaufanie certyfikatowi Caddy

Caddy wystawia certyfikat z wewnętrznego CA. Dopóki urządzenie go nie zna,
przeglądarka pokaże ostrzeżenie, a kamera może odmówić działania. Eksport
certyfikatu głównego:

```bash
docker compose -f docker-compose.yml -f docker-compose.https.yml \
  exec -T caddy cat /data/caddy/pki/authorities/local/root.crt > zsl-caddy-root.crt
```

Instalacja na urządzeniach:

| System | Kroki |
| --- | --- |
| Windows | dwuklik `zsl-caddy-root.crt` → Zainstaluj certyfikat → **Zaufane główne urzędy certyfikacji** → Komputer lokalny |
| Android | Ustawienia → Zabezpieczenia → Szyfrowanie i dane logowania → Zainstaluj certyfikat → **Certyfikat CA** → wybierz plik |
| iOS/iPadOS | wyślij plik na urządzenie i zainstaluj profil, potem Ustawienia → Ogólne → Informacje → Ustawienia zaufania certyfikatów → włącz pełne zaufanie |
| Linux | `sudo cp zsl-caddy-root.crt /usr/local/share/ca-certificates/zsl-caddy.crt && sudo update-ca-certificates` |
| Firefox | osobny magazyn: Ustawienia → Prywatność i bezpieczeństwo → Certyfikaty → Wyświetl certyfikaty → Urzędy → Importuj |

Po instalacji CA otwórz `https://<IP>` bez ostrzeżenia; kamera i
`getUserMedia` działają normalnie.

### Bez własnego CA

Alternatywy dla HTTPS: Nginx z certyfikatem z `mkcert`, odwrotny proxy na
istniejącym hoście albo Cloudflare Tunnel (tryb `tunnel`, prawdziwy
certyfikat, działa też poza siecią lokalną — patrz
[docs/DEPLOYMENT.md](DEPLOYMENT.md)).

## 4. Przeglądarka wymusza HTTPS

Nowoczesne przeglądarki przy wpisaniu samego adresu (`<ADRES-SYSTEMU>:11000`)
najpierw próbują `https://`. Ten serwer nie obsługuje TLS, więc próba kończy
się błędem i wygląda to jak „nie ma połączenia”.

Rozwiązania:

1. wpisz pełny adres z protokołem: `http://<ADRES-SYSTEMU>:11000`,
2. albo postaw HTTPS z §3 i korzystaj z `https://`,
3. w Chrome wyłącz „Always use secure connections” (Ustawienia →
   Prywatność i bezpieczeństwo → Bezpieczeństwo), jeśli nie chcesz HTTPS
   w laboratorium.

Aplikacja **nie ustawia HSTS w trybie `local`**, więc jedna udana wizyta po
HTTP nie „przykleja” przeglądarki do HTTPS.

## 5. Co gdzie leży

| Ścieżka | Zawartość |
| --- | --- |
| wolumen `<projekt>_zsl-data` → `/data` | `baza.db` (+ WAL/SHM), `jwt.key`, `config.json`, `admin_start.txt`, `backups/` |
| `/data/frontend` | dowiązanie do `/app/frontend` (tworzone przez entrypoint) |
| `/app/zsl-order` | binarka (tylko do odczytu, `root:root`) |
| `/app/frontend` | statyczne HTML/CSS/JS w obrazie |

Wolumen jest tworzony automatycznie i inicjowany zawartością `/data` z
obrazu. Użytkownik w kontenerze ma UID/GID **10001**; pliki bazy i kluczy
powstają z uprawnieniami `0600`.

## 6. Konfiguracja

Ustawienia przekazuje się zmiennymi środowiskowymi w `docker-compose.yml`
(sekcja `environment`) lub w pliku `.env` obok `docker-compose.yml`.

| Zmienna | Domyślnie | Znaczenie |
| --- | --- | --- |
| `BIND_ADDR` | `0.0.0.0:3000` | adres nasłuchu wewnątrz kontenera |
| `DEPLOY_MODE` | `local` | `local` (sieć szkolna) lub `tunnel` (Cloudflare) |
| `PUBLIC_URL` | brak | wymagane w trybie `tunnel`, np. `https://zsl.example.com` |
| `RETENTION_ORDERS_DAYS` | `0` | 0 = bez usuwania starych zamówień |
| `RETENTION_AUDIT_DAYS` | `90` | retencja dziennika audytu |
| `SESSION_HOURS` | `12` | ważność sesji |
| `CLIENT_IP_HEADER` | brak | `X-Forwarded-For` lub `X-Real-IP`; honorowany tylko od `TRUSTED_PROXIES` |
| `TRUSTED_PROXIES` | `127.0.0.1,::1` | adresy/CIDR proxy, którym wolno przekazać adres klienta |
| `LOG_CLIENT_IP` | brak | `1` dopisuje adres klienta do logów żądań (diagnostyka sieci) |
| `RUST_LOG` | `info,sqlx=warn,hyper=warn,axum=warn` | poziom logów |

Po zmianie: `docker compose up -d` (Compose odtworzy kontener).

Wzorem pozostałych zmiennych jest `.env.example`; pełny opis:
[docs/DEPLOYMENT.md](DEPLOYMENT.md) i [docs/INSTRUKCJA.md](INSTRUKCJA.md).

## 7. Firewall i sieć

- Docker publikuje port przez własny NAT (łańcuch `DOCKER`), **z pominięciem
  reguł UFW**. `sudo ufw allow 3000/tcp` nie jest potrzebne, a
  `sudo ufw deny 3000` tego ruchu nie zablokuje.
- Port publikuje wyłącznie host: `ports: ["11000:3000"]` w
  `docker-compose.yml`, a HTTPS `["10000:443"]` w nakładce HTTPS. Zmiana
  lewej strony (np. `"12000:3000"`) zmienia adres dla klientów.
- Chcesz, żeby port był dostępny tylko z tego komputera (np. za lokalnym
  proxy): `"127.0.0.1:11000:3000"`.
- Adres `172.x` widoczny w logu startowym (`Adres wewnątrz kontenera`) to
  adres w sieci Dockera — **nie** jest adresem dla uczniów.
- Jeśli z innego urządzenia nie ma połączenia: sprawdź, czy telefon jest w
  tej samej podsieci, czy router nie ma izolacji klientów (AP isolation) i
  czy nie jest włączony VPN.

## 8. Kopie zapasowe

```bash
docker compose exec zsl-order /app/zsl-order backup
```

Kopia trafia do `/data/backups` w wolumenie. Wyciągnięcie na hosta:

```bash
docker compose cp zsl-order:/data/backups ./backups
```

Harmonogram (cron hosta, codziennie o 3:30):

```cron
30 3 * * * cd /sciezka/do/projektu && docker compose exec -T zsl-order /app/zsl-order backup >> /var/log/zsl-backup.log 2>&1
```

Odtworzenie bazy z kopii: zatrzymaj kontener, podmienisz plik w wolumenie
(`docker compose cp` w obie strony), uruchom ponownie. Szczegóły:
[docs/DEPLOYMENT.md](DEPLOYMENT.md).

## 9. Aktualizacja i reset

Aktualizacja do nowej wersji kodu:

```bash
git pull
docker compose build --pull
docker compose up -d
```

Dane w wolumenie pozostają; migracje bazy wykonują się przy starcie.
Rollback: przebuduj obraz z poprzedniego taga/commita.

Całkowity reset (uwaga: usuwa bazę, klucz JWT i konta):

```bash
docker compose down -v
docker compose up -d --build
```

## 10. Logi i diagnostyka

```bash
docker compose logs -f                      # logi na żywo
docker compose ps                           # status i healthcheck
docker compose exec zsl-order curl -fsS http://127.0.0.1:3000/api/healthz
docker compose exec zsl-order sh -c 'ls -l /data'
```

Diagnostyka „czy ruch z sieci dociera”: ustaw `LOG_CLIENT_IP: "1"` w
`docker-compose.yml`, uruchom ponownie i obserwuj logi — w każdym żądaniu
pojawi się `"client_ip"`. Domyślnie pole ma wartość `-` (adresy IP nie są
przechowywane).

## 11. Bez Compose

```bash
docker build -t zsl-order:1.0.0 .
docker run -d --name zsl-order \
  -p 11000:3000 \
  -v zsl-data:/data \
  --restart unless-stopped \
  zsl-order:1.0.0

docker exec zsl-order cat /data/admin_start.txt
```

## 12. Ograniczenia

- **Tryb `local` za reverse proxy**: aplikacja widzi adres kontenera Caddy,
  więc limity nieudanych logowań per IP są wspólne dla wszystkich klientów.
  Limity per konto działają normalnie.
- **Bind mount zamiast wolumenu**: przy `-v /host/sciezka:/data` pliki nie
  dostaną `0600` w rozumieniu systemu hosta (Windows/macOS), a katalog musi
  być zapisywalny dla UID 10001. Zalecany jest wolumen nazwany.
- **Kamera QR**: wymaga HTTPS z zaufanym certyfikatem (jak w §3) albo
  `localhost`. Po HTTP z adresu IP przeglądarka zablokuje `getUserMedia`.
- **Jedna instancja**: SQLite i wolumen nie są przeznaczone do pracy wielu
  replik jednocześnie.
