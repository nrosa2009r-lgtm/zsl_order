# Instrukcja obsługi — ZSL Order

Instrukcja dla administratora, kuchni i uczniów. Opisuje instalację,
konfigurację, codzienną pracę i utrzymanie systemu zamawiania posiłków.

> **Zasada bezpieczeństwa:** w tej instrukcji nie zapisuj prawdziwych
> haseł, domen, adresów IP, e-maili ani danych uczniów. W przykładach
> używaj placeholderów: `<DOMENA>`, `<ADRES-SERWERA>`, `<LOGIN>`,
> `<HASŁO>`, `<TOKEN>`. Prawdziwe sekrety trzymaj w menedżerze haseł
> albo w plikach z uprawnieniami `0600`, nigdy w repozytorium.

Spis treści:

1. [Wymagania i przygotowanie serwera](#1-wymagania-i-przygotowanie-serwera)
2. [Instalacja i pierwsze uruchomienie](#2-instalacja-i-pierwsze-uruchomienie)
3. [Konfiguracja wstępna](#3-konfiguracja-wstępna)
4. [Zakładanie kont](#4-zakładanie-kont)
5. [Obsługa dla ucznia](#5-obsługa-dla-ucznia)
6. [Obsługa dla kuchni](#6-obsługa-dla-kuchni)
7. [Obsługa dla administratora](#7-obsługa-dla-administratora)
8. [Kopie zapasowe, odtwarzanie i aktualizacje](#8-kopie-zapasowe-odtwarzanie-i-aktualizacje)
9. [Cloudflare Tunnel i Cloudflare Access](#9-cloudflare-tunnel-i-cloudflare-access)
10. [Rozwiązywanie problemów i FAQ](#10-rozwiązywanie-problemów-i-faq)
11. [Checklista bezpieczeństwa przed produkcją](#11-checklista-bezpieczeństwa-przed-produkcją)

Ściągawki do wydruku:

- [Ściągawka ucznia](SCIAGAWKA-UCZEN.md)
- [Ściągawka kuchni](SCIAGAWKA-KUCHNIA.md)

---

## 1. Wymagania i przygotowanie serwera

### 1.1. Sprzęt i system

Minimalne, sprawdzone wymagania dla jednej stołówki:

| Element | Minimum | Zalecane |
| --- | --- | --- |
| Procesor | 1 vCPU | 2 vCPU |
| RAM | 512 MB | 1 GB |
| Dysk | 5 GB | 20 GB + miejsce na kopie |
| System | Linux x86_64 | Debian 12 / Ubuntu 24.04 LTS |
| Sieć | stały adres w sieci szkolnej | rezerwacja DHCP + UPS |

Aplikacja nie wymaga zewnętrznej bazy danych — używa SQLite w trybie WAL.
Jedna instancja obsługuje jedną stołówkę. Nie uruchamiaj kilku kopii
usługi na tym samym katalogu danych.

### 1.2. Przygotowanie systemu

Wykonaj na serwerze jako użytkownik z uprawnieniami `sudo`:

```bash
sudo apt update
sudo apt install -y build-essential pkg-config libssl-dev sqlite3 curl jq age rsync
```

Zainstaluj Rust (jeśli budujesz na serwerze) albo przygotuj binarkę na
innym komputerze:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
rustc --version
```

Ustaw poprawną strefę czasu i synchronizację zegara — terminy zamówień,
retencja i kopie zależą od czasu `Europe/Warsaw`:

```bash
sudo timedatectl set-timezone Europe/Warsaw
timedatectl status
sudo systemctl enable --now systemd-timesyncd
```

### 1.3. Konto usługi i katalogi

System używa jednego konta systemowego `zsl-order`. Nie uruchamiaj
aplikacji jako `root` ani jako zwykły użytkownik szkolny.

```bash
sudo groupadd --system zsl-order
sudo useradd --system --gid zsl-order --home-dir /var/lib/zsl-order \
  --shell /usr/sbin/nologin zsl-order

sudo install -d -o root -g root -m 0755 /opt/zsl-order
sudo install -d -o zsl-order -g zsl-order -m 0700 /var/lib/zsl-order
sudo install -d -o root -g zsl-order -m 0750 /etc/zsl-order
```

Układ katalogów:

```text
/opt/zsl-order/            kod, binarka i frontend (tylko odczyt)
/var/lib/zsl-order/        baza, konfiguracja runtime, kopie (0700)
/etc/zsl-order/            plik środowiskowy usługi (0640)
```

### 1.4. Wybór trybu dostępu

- **`local`** — aplikacja działa w sieci szkolnej po HTTP. Najprostszy
  start. Kamera QR na innym urządzeniu **nie zadziała** bez HTTPS;
  używaj czytnika USB albo wdróż HTTPS (rozdział 9).
- **`tunnel`** — aplikacja jest schowana za Cloudflare Tunnel, który
  kończy TLS. Daje HTTPS bez otwierania portów. Wymaga domeny i konta
  Cloudflare.

Nie mieszaj trybów w jednym wdrożeniu. Tryb zmienia się w `.env`
i wymaga restartu usługi.

### 1.5. Firewall

W trybie `local` otwórz tylko port aplikacji w sieci szkolnej, np.:

```bash
sudo ufw allow from <PODSIEC-SZKOLNA> to any port 3000 proto tcp
sudo ufw enable
sudo ufw status
```

W trybie `tunnel` **nie otwieraj** portu aplikacji. Jedynym klientem ma
być `cloudflared` na tym samym hoście. Dostęp administracyjny do SSH
ogranicz do sieci zaufanej lub VPN.

---

## 2. Instalacja i pierwsze uruchomienie

### 2.1. Budowanie

Na komputerze deweloperskim albo na serwerze:

```bash
git clone https://github.com/nrosa2009r-lgtm/zsl-order.git
cd zsl-order
cargo build --release --locked
```

Binarka to `target/release/zsl-order`. Jeśli budujesz na innym
komputerze, skopiuj na serwer wyłącznie binarkę i katalog `frontend/`.

### 2.2. Instalacja plików

```bash
sudo install -o root -g root -m 0755 target/release/zsl-order /opt/zsl-order/zsl-order
sudo rsync -a --delete frontend/ /opt/zsl-order/frontend/
sudo chown -R root:root /opt/zsl-order/frontend
sudo ln -s /opt/zsl-order/frontend /var/lib/zsl-order/frontend
```

`frontend/` musi być widoczny względem katalogu roboczego usługi
(`/var/lib/zsl-order`). Symlink jest wygodniejszy przy aktualizacjach niż
kopiowanie plików.

### 2.3. Konfiguracja startowa

Skopiuj przykłady:

```bash
sudo -u zsl-order cp /opt/zsl-order/config.example.json /var/lib/zsl-order/config.json
sudo install -o root -g zsl-order -m 0640 /opt/zsl-order/.env.example /etc/zsl-order/zsl-order.env
```

Edytuj `/etc/zsl-order/zsl-order.env`:

```dotenv
DEPLOY_MODE=local
BIND_ADDR=0.0.0.0:3000
COOKIE_SECURE=auto
PUBLIC_URL=
```

Ustaw `BIND_ADDR` na adres, który ma być dostępny w sieci szkolnej.
Aby ograniczyć dostęp wyłącznie do samego serwera, użyj
`127.0.0.1:3000`.

> Nie wpisuj w `.env` haseł ani tokenów. Sekret JWT (`jwt.key`) tworzy
> sama aplikacja przy pierwszym starcie.

### 2.4. Usługa systemd

Skopiuj jednostkę z repozytorium:

```bash
sudo install -o root -g root -m 0644 deploy/systemd/zsl-order.service /etc/systemd/system/zsl-order.service
sudo systemctl daemon-reload
sudo systemctl enable --now zsl-order
sudo systemctl status zsl-order
```

Sprawdź logi i zdrowie aplikacji:

```bash
sudo journalctl -u zsl-order -n 50 --no-pager
curl -fsS http://127.0.0.1:3000/api/healthz
```

### 2.5. Odczyt hasła startowego

Przy pierwszym starcie, **tylko gdy w bazie nie ma żadnego admina**,
aplikacja tworzy losowe konto administratora i zapisuje dane w pliku
`/var/lib/zsl-order/admin_start.txt` z uprawnieniami `0600`.

```bash
sudo -u zsl-order cat /var/lib/zsl-order/admin_start.txt
```

Plik:

- nie jest serwowany przez HTTP (żądanie `/admin_start.txt` zwraca `404`),
- nie trafia do `journalctl`,
- jest usuwany automatycznie po zmianie hasła,
- trzeba usunąć ręcznie, jeśli zmiana hasła się nie powiedzie:

```bash
sudo -u zsl-order rm -f /var/lib/zsl-order/admin_start.txt
```

### 2.6. Pierwsze logowanie administratora

1. Otwórz `http://<ADRES-SERWERA>:3000/admin`.
2. Zaloguj się loginem i hasłem z `admin_start.txt`.
3. System wymusi zmianę hasła. Wpisz nowe hasło (dla admina minimum
   12 znaków; nie może zawierać loginu) i zapisz je w menedżerze haseł.
4. Po zmianie sprawdź, że plik zniknął:

```bash
sudo test ! -e /var/lib/zsl-order/admin_start.txt && echo "OK: usunięty"
```

5. Sprawdź, że panel admina działa, a w dzienniku audytu jest wpis
   o zmianie hasła.

Jeśli nie widzisz panelu, przejdź do rozdziału 10.

---

## 3. Konfiguracja wstępna

Konfiguracja dzieli się na trzy warstwy:

1. **`.env`** — ustawienia wdrożeniowe (sieć, tryb, ścieżki, retencja).
   Czytane przez systemd; wymagają restartu usługi.
2. **`config.json`** — ustawienia bazy, sesji, backupu i limiterów.
   Czytane przy starcie; wymagają restartu usługi.
3. **Ustawienia w panelu admina** — nazwa stołówki, menu, terminy i limity
   biznesowe. Działają natychmiast, bez restartu.

### 3.1. Plik `.env` — każde pole

Plik: `/etc/zsl-order/zsl-order.env` (`root:zsl-order`, `0640`).

| Pole | Domyślnie | Znaczenie |
| --- | --- | --- |
| `DEPLOY_MODE` | `local` | `local` albo `tunnel`. W `tunnel` wymagany `PUBLIC_URL`, a `BIND_ADDR` musi być pętlą. |
| `BIND_ADDR` | `0.0.0.0:3000` | Adres nasłuchu. `127.0.0.1:3000` ogranicza do localhost; `0.0.0.0:3000` otwiera sieć. |
| `TRUSTED_PROXIES` | `127.0.0.1,::1` | Lista IP/CIDR proxy, którym wolno przekazać `CF-Connecting-IP`. Tylko tryb `tunnel`. |
| `PUBLIC_URL` | brak | Publiczny adres bez ścieżki, np. `https://<DOMENA>`. Wymagany w `tunnel`. |
| `COOKIE_SECURE` | `auto` | `auto`, `true` lub `false`. `auto` włącza `Secure`, gdy `PUBLIC_URL` jest HTTPS; w `tunnel` zawsze `true`. |
| `BACKUP_DIR` | `backups` | Katalog kopii: względny wobec `/var/lib/zsl-order` albo absolutny. Zastępuje `backup_dir`. |
| `BACKUP_KEEP_DAYS` | `30` | Retencja kopii w dniach. Zawsze zostaje co najmniej 7 najnowszych. |
| `RETENTION_ORDERS_DAYS` | `0` | Po ilu dniach usuwać stare zamówienia. `0` = nigdy. Ustaw świadomie, np. `365`. |
| `RETENTION_IDEMPOTENCY_DAYS` | `7` | Po ilu dniach zerować klucze idempotencji. `0` = nigdy. |
| `RETENTION_AUDIT_DAYS` | `90` | Po ilu dniach czyścić audyt. `0` = nigdy. |
| `RETENTION_RUN_AT` | `04:00` | Stała pora nocnego czyszczenia (`HH:MM`, `Europe/Warsaw`). |

Priorytet: `.env` zastępuje zgodnościowe pola `config.json`. Jeśli pole
występuje w obu miejscach, wygrywa `.env`.

### 3.2. Plik `config.json` — każde pole

Plik: `/var/lib/zsl-order/config.json` (`zsl-order:zsl-order`, `0600`).

| Pole | Domyślnie | Znaczenie |
| --- | --- | --- |
| `db_url` | `sqlite://baza.db` | Ścieżka bazy względem katalogu roboczego. Nie zmieniaj bez przeniesienia pliku. |
| `max_connections` | `16` | Pula połączeń SQLite. Dla małej stołówki wystarczy `4`–`16`. |
| `session_hours` | `12` | Ważność sesji ciasteczkowej w godzinach. |
| `backup_dir` | `backups` | Katalog kopii (nadpisywany przez `BACKUP_DIR`). |
| `backup_keep_days` | `30` | Retencja kopii (nadpisywana przez `BACKUP_KEEP_DAYS`). |
| `retention_orders_days` | `0` | Retencja zamówień; `0` = bez czyszczenia. |
| `retention_idempotency_days` | `7` | Retencja kluczy idempotencji; `0` = bez czyszczenia. |
| `retention_audit_days` | `90` | Retencja audytu; `0` = bez czyszczenia. |
| `retention_run_at` | `04:00` | Pora czyszczenia; format `HH:MM`. |
| `login_max_failures` | `10` | Ile błędnych logowań na konto blokuje konto. |
| `login_ip_max_failures` | `300` | Ile błędnych logowań z jednego IP blokuje IP. |
| `login_window_minutes` | `10` | Okno liczenia błędów logowania. |
| `login_lockout_minutes` | `15` | Czas blokady po przekroczeniu limitu. |
| `pickup_max_failures` | `8` | Ile błędnych kodów QR/krótkich blokuje konto personelu. |
| `pickup_ip_max_failures` | `100` | Ile błędnych kodów z jednego IP blokuje IP. |
| `pickup_window_minutes` | `10` | Okno liczenia błędów odbioru. |
| `pickup_lockout_minutes` | `15` | Czas blokady limitera odbioru. |

Po zmianie `config.json` lub `.env`:

```bash
sudo systemctl restart zsl-order
sudo journalctl -u zsl-order -n 30 --no-pager
```

### 3.3. Ustawienia w panelu — menu i ceny

W panelu admina przejdź do zakładki **Menu**. Jedna pozycja ma pola:

| Pole | Znaczenie | Uwagi |
| --- | --- | --- |
| `name` | nazwa pozycji | 1–120 znaków |
| `description` | opis | opcjonalny |
| `price_cents` | cena w groszach | `1250` = 12,50 zł |
| `category` | kategoria | domyślnie `Dania`; grupuje menu ucznia |
| `allergens` | alergeny | tekst, np. `gluten, mleko` |
| `available` | dostępność | `false` ukrywa pozycję przed uczniami |
| `sort_order` | kolejność | mniejsza liczba = wyżej |

Pozycje można edytować i ukrywać. Ukrycie nie usuwa historii zamówień.
Ceny zawsze liczy serwer z bazy — uczeń nie może ich zmienić.

### 3.4. Godziny, terminy i limity

W panelu admina **Ustawienia**:

| Pole | Domyślnie | Zakres | Znaczenie |
| --- | --- | --- | --- |
| `cafeteria_name` | `Stołówka ZSŁ` | 1–80 znaków | nazwa widoczna w panelach |
| `ordering_enabled` | `true` | — | wyłączenie blokuje nowe zamówienia |
| `payment_info` | `Płatność przy odbiorze: gotówka lub karta.` | do 300 znaków | informacja dla ucznia |
| `max_open_orders` | `3` | 1–10 | limit otwartych zamówień jednego ucznia |
| `max_order_items` | `20` | 1–50 | limit pozycji w jednym zamówieniu |
| `cancel_window_minutes` | `30` | 0–1440 | jak długo uczeń może anulować; `0` = bez limitu |
| `pickup_limit_minutes` | `120` | 0–720 | po ilu minutach od `ready` zamówienie staje się nieodebrane; `0` = wyłączone |
| `max_uncollected` | `0` | 0–20 | ile nieodebranych blokuje kolejne zamówienia; `0` = brak blokady |

Semantyka:

- Anulowanie działa tylko dla zamówienia w statusie `new` i w oknie
  `cancel_window_minutes` liczonym od złożenia.
- „Nieodebrane” ustawia się automatycznie, gdy zamówienie jest `ready`
  dłużej niż `pickup_limit_minutes`. Sprawdzenie odbywa się przy starcie
  i cyklicznie w tle.
- Po przekroczeniu `max_uncollected` uczeń nie złoży nowego zamówienia,
  dopóki administrator nie zdejmie blokady (rozdział 7.6).
- Ustawienia działają natychmiast; nie wymagają restartu.

### 3.5. Retencja danych

Nocne czyszczenie uruchamia się o `RETENTION_RUN_AT` (domyślnie `04:00`).
Kopia zapasowa startuje o `03:15` i może być losowo opóźniona do 15 minut,
czyli najpóźniej o `03:30`. Dzięki temu kopia kończy się przed
czyszczeniem; między nimi zostaje co najmniej 30 minut.

- `retention_orders_days = 0` oznacza, że zamówienia **nie** są usuwane.
  Włącz retencję świadomie, po ustaleniu polityki szkoły.
- Audyt domyślnie żyje 90 dni, klucze idempotencji 7 dni.
- **Kopie zapasowe zawierają dane starsze niż retencja.** Backup jest
  migawką z chwili wykonania; dane usunięte później z bazy nadal mogą
  być w starszych kopiach. Retencja kopii i szyfrowanie to część tej
  samej polityki.

Sprawdzenie, kiedy zadziała czyszczenie:

```bash
sudo journalctl -u zsl-order --since today | grep -Ei 'retencja|retention'
```

---

## 4. Zakładanie kont

Konta zakłada wyłącznie administrator. Nie ma publicznej rejestracji.
System nie przechowuje imion, nazwisk, e-maili ani numerów PESEL.

### 4.1. Import klas z CSV

Format: `klasa;liczba` — jedna grupa w wierszu, bez danych osobowych.

```csv
# komentarz
1A;30
2B;25
3C;28
```

Zasady:

- klasa/grupa: 1–16 znaków (litery ASCII, cyfry, `-`, `_`),
- 1–100 kont na grupę, maksymalnie 200 kont na jedno żądanie,
- wiersze zaczynające się od `#` są komentarzami, nagłówek jest pomijany,
- separatorem może być `;` lub `,`.

W panelu: **Konta → Import CSV**. Po imporcie system pokaże listę kont
z hasłami **tylko ten jeden raz**. Od razu wydrukuj lub zapisz hasła w
bezpieczny sposób. Audyt zapisuje tylko liczbę kont i grup — nigdy hasła.

### 4.2. Generowanie pojedynczych kont

W panelu **Konta → Wygeneruj** wybierz rolę (`student`, `kitchen`,
`admin`) i liczbę kont. Hasła są widoczne wyłącznie w odpowiedzi po
utworzeniu. Nowe konto ma wymuszoną zmianę hasła przy pierwszym logowaniu.

### 4.3. Wydruk kart z hasłami

Praktyczna procedura:

1. Wydrukuj karty **jednostronnie** — na odwrocie nie może być innej
   kartki z danymi.
2. Rozetnij i zabezpiecz kartki (koperta, zaklejony stos, teczka
   wychowawcy).
3. Każda karta zawiera login, hasło startowe i adres panelu.
4. Hasło jest **jednorazowe** — uczeń zmienia je przy pierwszym
   logowaniu. Karta po wykorzystaniu nie pozwala na ponowne wejście.
5. Nie wysyłaj haseł e-mailem, na grupowy czat ani do dziennika
   elektronicznego w treści wiadomości. Jeśli musisz przekazać zdalnie,
   użyj indywidualnego, bezpiecznego kanału.
6. Po rozdaniu kart odnotuj, które konta wydano, i zniszcz nadmiarowe
   wydruki (najlepiej przez zniszczarkę).

### 4.4. Reset hasła

W panelu **Konta** znajdź konto i wybierz **Reset hasła**. System wyświetli
nowe hasło jednorazowo. Przekaż je uczniowi w bezpieczny sposób; przy
następnym logowaniu wymuszona będzie zmiana.

Reset hasła unieważnia wszystkie sesje i tokeny JWT danego konta. Uczeń
musi zalogować się ponownie.

### 4.5. Blokada i dezaktywacja konta

- **Dezaktywacja** (`active = false`) blokuje logowanie i natychmiast
  unieważnia wszystkie sesje oraz tokeny JWT.
- **Zmiana roli** również unieważnia sesje — użytkownik musi zalogować
  się ponownie z nowymi uprawnieniami.
- **Twarde usunięcie** konta usuwa konto, sesje i historię zamówień.
  Stosuj ostrożnie; nie da się tego cofnąć.
- Administrator nie może odebrać uprawnień własnemu kontu — system
  chroni przed przypadkowym zablokowaniem się.

### 4.6. Unieważnianie sesji

W panelu kont wybierz **Unieważnij sesje**. To wylogowuje użytkownika ze
wszystkich urządzeń i unieważnia tokeny JWT. Użyj, gdy podejrzewasz
wyciek hasła lub zgubiono telefon/tablet.

### 4.7. Zdejmowanie blokady nieodebranych

Blokada `max_uncollected` dotyczy ucznia, nie całej klasy. Administrator
zdejmuje ją na karcie konta (**Zdejmij blokadę nieodebranych**). Historia
nieodebranego zamówienia pozostaje w bazie i raporcie.

---

## 5. Obsługa dla ucznia

### 5.1. Logowanie

1. Otwórz `http://<ADRES-SERWERA>:3000/student` (lub adres HTTPS podany
   przez szkołę).
2. Wpisz login i hasło startowe z karty.
3. Przy pierwszym logowaniu zmień hasło na własne (minimum 10 znaków;
   nie może zawierać loginu ani być zbyt popularne). Nowe hasło
   zapamiętaj — nie ma odzyskiwania hasła przez e-mail.
4. Jeśli hasło zostanie zgubione, zgłoś się do administratora/stołówki.
   Administrator może je zresetować.

Nie udostępniaj loginu i hasła kolegom. Konto jest powiązane z Twoimi
zamówieniami i historią płatności.

### 5.2. Zamawianie

1. W zakładce **Menu** wybierz pozycje i ilości (maksymalnie 10 szt.
   jednej pozycji, do `max_order_items` pozycji).
2. Sprawdź alergeny i dostępność.
3. Wybierz sposób płatności przy odbiorze: **gotówka** albo **karta**.
4. Opcjonalnie podaj godzinę odbioru w formacie `HH:MM`.
5. Kliknij **Zamów**. System policzy kwotę po stronie serwera.

Po złożeniu zobaczysz krótki kod odbioru (np. `K7M2QP`) oraz kod QR.
Zapisz/pozostaw kod — kuchnia odszuka zamówienie po kodzie lub QR.

### 5.3. Płatność

Płatność następuje przy odbiorze. Sposób wybrany w zamówieniu
(gotówka/karta) widzi kuchnia. Przy wydaniu kuchnia oznacza zamówienie
jako opłacone; jeśli było nieopłacone, system zrobi to automatycznie.

### 5.4. Kod QR

- QR zawiera wyłącznie losowy token — **bez** loginu, danych ucznia,
  ceny i adresu. Sam kod nie daje dostępu bez zalogowanego konta
  personelu.
- QR działa tylko dla **dzisiejszego** zamówienia w statusie `ready`.
- Po wydaniu kod jest jednorazowo zużyty; ponowne skanowanie zwróci błąd.
- Nie publikuj QR w mediach społecznościowych. Traktuj go jak bilet.

### 5.5. Anulowanie

Zamówienie możesz anulować tylko wtedy, gdy:

- ma status `new` (nie zostało jeszcze przyjęte do przygotowania),
- nie minął termin `cancel_window_minutes` od złożenia.

Po anulowaniu kod QR przestaje działać. Jeśli przycisk anulowania jest
nieaktywny, poproś kuchnię o pomoc — po rozpoczęciu przygotowania
anulowanie nie jest możliwe.

### 5.6. Statusy widoczne dla ucznia

| Status | Znaczenie |
| --- | --- |
| `new` | przyjęte, czeka na przygotowanie |
| `preparing` | w przygotowaniu |
| `ready` | gotowe do odbioru |
| `completed` | wydane |
| `cancelled` | anulowane |
| `uncollected` | nieodebrane w terminie |

---

## 6. Obsługa dla kuchni

### 6.1. Logowanie i ekran

1. Otwórz `/kitchen` na tablecie/komputerze w kuchni.
2. Zaloguj się kontem roli `kitchen` (lub `admin`).
3. Zakładki: **Zamówienia**, **Odbiór QR**, **Menu**, **Raport**.

Kuchnia widzi i zmienia wyłącznie **dzisiejsze** zamówienia. Próba
zmiany zamówienia z innego dnia kończy się `403`.

### 6.2. Lista zamówień i statusy

- Domyślnie widoczne są zamówienia otwarte: `new`, `preparing`, `ready`.
- Przejście statusów: `new → preparing → ready → completed`.
- `completed` oznacza wydanie i automatycznie oznacza płatność jako
  opłaconą, jeśli była nieopłacona.
- Anulowane (`cancelled`) i nieodebrane (`uncollected`) są zamknięte —
  kuchnia nie może ich ponownie otworzyć.

Nie cofaj płatności bez zgody administratora. Kuchnia nie może cofnąć
oznaczenia „opłacone”; robi to wyłącznie administrator.

### 6.3. Odbiór QR — szybka procedura

1. Wejdź w zakładkę **Odbiór QR**. Pole **Kod odbioru** ma automatyczny
   fokus.
2. Zeskanuj QR kamerą, użyj czytnika USB/Bluetooth (działa jak
   klawiatura — wpisuje kod i naciska `Enter`) albo wpisz 6-znakowy kod
   ręcznie i naciśnij `Enter`.
3. System automatycznie wyszuka zamówienie i pokaże:
   - kod zamówienia, status, sposób i status płatności,
   - pozycje, kwotę do zapłaty,
   - jedną dużą akcję **Wydaj**.
4. Sprawdź kwotę i płatność. Jeśli zamówienie jest nieopłacone, przycisk
   **Wydaj** oznaczy je jako opłacone.
5. Po wydaniu usłyszysz sygnał sukcesu, poczujesz wibrację (jeśli
   urządzenie wspiera), a pole kodu odzyska fokus. Możesz od razu
   skanować następne zamówienie — bez klikania.

Błędy:

- **404** — kod nie istnieje. Sprawdź, czy nie ma literówki.
- **409** — zamówienie nie jest dzisiaj, nie jest jeszcze `ready`, zostało
  już wydane, anulowane lub nieodebrane. Komunikat wyjaśnia powód.
- **429** — zbyt wiele błędnych prób. Odczekaj czas podany w komunikacie
  (domyślnie 15 minut) i poproś administratora, jeśli blokada przeszkadza.

Sygnał błędu jest niższy i dłuższy; karta błędu jest czerwona. Po błędzie
pole kodu pozostaje aktywne.

### 6.4. Kamera a czytnik USB

Kamera w przeglądarce działa tylko w bezpiecznym kontekście: HTTPS albo
`localhost`. W trybie `local` po HTTP na tablecie kamera **nie zadziała**
— panel pokaże czytelny komunikat i wskaże wpisanie kodu ręcznie.

W kuchni preferuj **czytnik USB/Bluetooth** (klawiaturowy). Nie wymaga
HTTPS, uprawnień do kamery ani obsługi `BarcodeDetector`. Jeśli chcesz
kamerę, wdróż HTTPS w sieci lokalnej albo tryb `tunnel` (rozdział 9).

### 6.5. Zamówienia nieodebrane

Gdy zamówienie jest `ready` dłużej niż `pickup_limit_minutes`, system
oznacza je jako `uncollected`. Kuchnia widzi taki status, ale nie może
go zmienić. Administrator może:

- zdjąć blokadę u ucznia (jeśli limit `max_uncollected` blokuje kolejne
  zamówienia),
- skorygować zamówienie (np. oznaczyć jako wydane), jeśli doszło do
  pomyłki.

### 6.6. Menu

W zakładce **Menu** kuchnia może dodawać, edytować i ukrywać pozycje.
Ceny podawaj w groszach (`1250` = 12,50 zł). Pozycja niedostępna nie
pojawi się u ucznia, ale pozostaje w historii zamówień.

---

## 7. Obsługa dla administratora

### 7.1. Statystyki i aktywność

Panel **Przegląd** pokazuje liczbę kont, zamówień i sesji oraz wykresy
aktywności (zamówienia dziennie, utarg, rozkład godzinowy, statusy,
najczęściej zamawiane pozycje). Wykresy są rysowane lokalnie, bez CDN.

### 7.2. Konta

Zakładka **Konta** pozwala:

- filtrować po roli, statusie i fragmencie loginu/kodu,
- generować konta (`student`, `kitchen`, `admin`),
- importować uczniów z CSV (`klasa;liczba`),
- resetować hasła (jednorazowo),
- dezaktywować/aktywować konto,
- zmieniać rolę,
- unieważniać sesje,
- usuwać konto wraz z historią,
- eksportować metadane kont do CSV (bez haseł).

Po wygenerowaniu/imporcie/resecie **natychmiast** wydrukuj lub zapisz
hasła. Po zamknięciu komunikatu nie da się ich odzyskać.

### 7.3. Menu

Zakładka **Menu**: dodawanie, edycja, ukrywanie pozycji. Zmiany są
widoczne u uczniów natychmiast. Nie usuwaj pozycji, której historia jest
potrzebna w raportach — użyj `available = false`.

### 7.4. Ustawienia

Zakładka **Ustawienia** — opis pól w rozdziale 3.4. Po zmianie limitów
sprawdź wpływ na otwarte zamówienia:

- skrócenie `cancel_window_minutes` działa od razu,
- zwiększenie `pickup_limit_minutes` nie przywraca już oznaczonych
  nieodebranych,
- `ordering_enabled = false` blokuje nowe zamówienia, ale nie zamyka
  istniejących.

### 7.5. Raport dzienny i CSV

Zakładka **Raport**:

1. Wybierz datę.
2. Odczytaj podsumowanie: liczba zamówień, opłacone, nieopłacone,
   anulowane, nieodebrane, sumy gotówka/karta/razem.
3. Pobierz CSV przyciskiem **Eksport CSV**.

CSV ma BOM, separator `;` i jest zgodny z Excelem. Zawiera podział na
gotówkę, kartę i razem. Formuły w komórkach są neutralizowane apostrofem
— nie otwieraj CSV w narzędziach, które ignorują to zabezpieczenie.

Raport liczony jest z bazy dla wybranej doby `Europe/Warsaw`. Zamówienie
opłacone, a potem nieodebrane, jest liczone raz — jako nieodebrane.

### 7.6. Audyt

Zakładka **Audyt** pokazuje zdarzenia: logowania, zmiany statusów,
płatności, importy, resety haseł, zmiany ustawień, odblokowania. Audyt
nie zawiera haseł, tokenów sesji ani danych osobowych. Domyślnie jest
czyszczony po 90 dniach.

Do audytu trafiają też zdarzenia bezpieczeństwa (np. blokady limitera)
jako techniczne logi JSON w `journalctl`. Nie zawierają haseł ani ciał
żądań.

### 7.7. Zdejmowanie blokady nieodebranych

Na karcie konta wybierz **Zdejmij blokadę nieodebranych**. Blokada jest
zdejmowana bez usuwania historii zamówienia. Czynność trafia do audytu.

### 7.8. Uprawnienia administratora

Administrator widzi wszystko, ale:

- nie może odebrać uprawnień własnemu kontu,
- cofnięcie płatności wymaga roli `admin`,
- zmiana roli/dezaktywacja unieważnia sesje,
- twarde usunięcie konta jest nieodwracalne.

Do codziennej pracy używaj konta `kitchen`; konto `admin` trzymaj do
konfiguracji i sytuacji awaryjnych.

---

## 8. Kopie zapasowe, odtwarzanie i aktualizacje

### 8.1. Wbudowana kopia

```bash
sudo systemctl start zsl-order-backup.service
sudo journalctl -u zsl-order-backup.service -n 30 --no-pager
```

Kopia:

- powstaje przez `VACUUM INTO` (bezpieczne dla WAL),
- jest weryfikowana przez `PRAGMA integrity_check`,
- ma uprawnienia `0600`,
- trafia do `BACKUP_DIR` jako `zsl-order-RRRRMMDD-HHMMSS.db`,
- podlega retencji, ale zawsze zostaje co najmniej 7 najnowszych.

Ręczne sprawdzenie najnowszej kopii:

```bash
sudo -u zsl-order sh -c 'cd /var/lib/zsl-order && /opt/zsl-order/zsl-order backup --verify "$(ls -1t backups/zsl-order-*.db | head -n1)"'
```

### 8.2. Timer codzienny

Jednostki w `deploy/systemd/`:

- `zsl-order-backup.timer` — codziennie `03:15` + losowe opóźnienie do
  15 minut,
- `zsl-order-backup.service` — wykonuje kopię,
- `zsl-order-backup-failed.service` — zgłasza błąd przez `logger`.

Instalacja:

```bash
sudo install -o root -g root -m 0644 deploy/systemd/zsl-order-backup.service /etc/systemd/system/
sudo install -o root -g root -m 0644 deploy/systemd/zsl-order-backup.timer /etc/systemd/system/
sudo install -o root -g root -m 0644 deploy/systemd/zsl-order-backup-failed.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now zsl-order-backup.timer
systemctl list-timers 'zsl-order*'
```

Sprawdź, że pojedyncze uruchomienie kończy się sukcesem. Timer nie
zastępuje monitoringu — skonfiguruj alert na `zsl-order-backup-failed`
albo na brak nowych plików w katalogu kopii.

### 8.3. Kopie poza serwerem

Kopia na tym samym dysku nie chroni przed awarią hosta ani ransomware.
Po każdej kopii wysyłaj ją na drugie urządzenie:

```bash
rsync -a --delete /var/lib/zsl-order/backups/ <UŻYTKOWNIK>@<HOST-KOPII>:/<KATALOG>/
```

Zdalny katalog zabezpiecz osobnym kontem, kluczem SSH i szyfrowanym
dyskiem. Szyfruj kopie opuszczające serwer, np. `age`:

```bash
age -r <KLUCZ-PUBLICZNY> -o kopia.db.age kopia.db
```

Klucz prywatny trzymaj **poza serwerem** (sejf, menedżer haseł). Nie
umieszczaj go obok zaszyfrowanych kopii.

### 8.4. Odtwarzanie — krok po kroku

Aplikacja nie ma polecenia `restore`; odtwarzanie jest celowo ręczne.

1. Zatrzymaj usługę:

   ```bash
   sudo systemctl stop zsl-order
   ```

2. Zweryfikuj wybraną kopię:

   ```bash
   sudo -u zsl-order sh -c 'cd /var/lib/zsl-order && /opt/zsl-order/zsl-order backup --verify backups/zsl-order-RRRRMMDD-HHMMSS.db'
   ```

3. Odsuń obecną bazę i pliki WAL (nie usuwaj od razu):

   ```bash
   cd /var/lib/zsl-order
   sudo -u zsl-order mkdir -p restore-old
   sudo -u zsl-order mv baza.db "restore-old/baza.db.$(date +%F-%H%M%S)"
   for plik in baza.db-wal baza.db-shm; do
     [ -e "$plik" ] && sudo -u zsl-order mv "$plik" "restore-old/$plik.$(date +%F-%H%M%S)"
   done
   ```

4. Wstaw kopię i ustaw uprawnienia:

   ```bash
   sudo -u zsl-order cp backups/zsl-order-RRRRMMDD-HHMMSS.db baza.db
   sudo -u zsl-order chmod 600 baza.db
   ```

5. Uruchom usługę i sprawdź integralność:

   ```bash
   sudo systemctl start zsl-order
   sudo -u zsl-order sh -c 'cd /var/lib/zsl-order && /opt/zsl-order/zsl-order backup --verify baza.db'
   curl -fsS http://127.0.0.1:3000/api/healthz
   sudo journalctl -u zsl-order -n 50 --no-pager
   ```

6. Zaloguj się do panelu, sprawdź zamówienia, raport i audyt. Dopiero po
   potwierdzeniu poprawności usuń `restore-old/`.

Nowa binarka automatycznie zastosuje brakujące migracje `sqlx` przy
starcie. Migracje są addytywne; nie usuwają danych. Jeśli kopia pochodzi
ze starszej wersji, zachowaj `restore-old/` do czasu weryfikacji.

### 8.5. Aktualizacja aplikacji

1. Wykonaj kopię i zapisz jej nazwę:

   ```bash
   sudo systemctl start zsl-order-backup.service
   sudo journalctl -u zsl-order-backup.service -n 20 --no-pager
   ```

2. Zatrzymaj usługę:

   ```bash
   sudo systemctl stop zsl-order
   ```

3. Zachowaj poprzednią binarkę (rollback):

   ```bash
   sudo cp /opt/zsl-order/zsl-order /opt/zsl-order/zsl-order.prev
   ```

4. Wgraj nową binarkę i frontend:

   ```bash
   sudo install -o root -g root -m 0755 target/release/zsl-order /opt/zsl-order/zsl-order
   sudo rsync -a --delete frontend/ /opt/zsl-order/frontend/
   sudo chown -R root:root /opt/zsl-order/frontend
   ```

5. Uruchom i sprawdź:

   ```bash
   sudo systemctl start zsl-order
   sudo systemctl status zsl-order
   curl -fsS http://127.0.0.1:3000/api/healthz
   sudo journalctl -u zsl-order -n 50 --no-pager
   ```

6. Sprawdź migracje:

   ```bash
   sqlite3 /var/lib/zsl-order/baza.db 'SELECT version, description FROM _sqlx_migrations ORDER BY version;'
   ```

7. Zaloguj się i wykonaj krótki test: menu, zamówienie testowe, skan QR,
   raport. Usuń konto testowe.

Jeśli nowa wersja nie startuje, wróć do `zsl-order.prev`, uruchom usługę
i przeanalizuj logi. Nie usuwaj kopii bazy.

### 8.6. Migracje

- Migracje znajdują się w `migrations/` i są wykonywane przy starcie.
- Wersje są zapisane w `_sqlx_migrations`.
- Nie edytuj ręcznie tabel ani `_sqlx_migrations`, jeśli nie wiesz, co
  robisz.
- Przed każdą aktualizacją zrób kopię i przetestuj migrację na kopii.
- Migracja bazowa używa `IF NOT EXISTS`, więc działa też na starszych
  bazach bez tabeli migracji.

---

## 9. Cloudflare Tunnel i Cloudflare Access

Ta sekcja zawiera **rekomendacje**. Konfigurację panelu Cloudflare
wykonujesz samodzielnie; aplikacja nie zastępuje Access, WAF ani rate
limitingu Cloudflare.

### 9.1. Kiedy używać

- Potrzebujesz HTTPS dla kamery QR na tabletach w sieci szkolnej.
- Chcesz udostępnić panel poza siecią szkolną bez otwierania portów.
- Nie chcesz zarządzać własnym certyfikatem TLS.

Tunnel to rozwiązanie **awaryjne/zdalne**. Ruch wychodzi poza sieć
lokalną, więc dostęp musi być ograniczony przez Cloudflare Access.

### 9.2. Architektura

```text
przeglądarka --HTTPS--> Cloudflare --tunel--> cloudflared --> 127.0.0.1:3000
```

Aplikacja nasłuchuje wyłącznie na pętli. Cloudflare kończy TLS, więc
aplikacja widzi HTTP od zaufanego proxy.

### 9.3. Konfiguracja aplikacji

`/etc/zsl-order/zsl-order.env`:

```dotenv
DEPLOY_MODE=tunnel
BIND_ADDR=127.0.0.1:3000
TRUSTED_PROXIES=127.0.0.1,::1
PUBLIC_URL=https://<DOMENA>
COOKIE_SECURE=auto
```

W tym trybie:

- `BIND_ADDR` musi być `127.0.0.1` lub `::1`,
- `PUBLIC_URL` jest wymagany; `Host` i `Origin` muszą wskazywać ten
  adres,
- `COOKIE_SECURE` jest zawsze `true`,
- HSTS jest dodawany przez aplikację,
- `CF-Connecting-IP` jest honorowany tylko od adresów z
  `TRUSTED_PROXIES`.

Nie otwieraj portu aplikacji w firewallu. Jedynym klientem ma być
`cloudflared` na tym samym hoście.

### 9.4. Cloudflare Access — zalecenia

1. Obejmij politykami Access co najmniej:
   - `/admin` i `/api/admin/*`,
   - `/kitchen` i `/api/staff/*`,
   - `/api/auth/token`.
2. Zezwalaj tylko konkretnym osobom/grupom (najlepiej przez szkolne SSO
   lub listę e-maili służbowych). Nie używaj „Everyone”.
3. Włącz MFA dla kont administracyjnych.
4. Dla automatyzacji używaj Service Tokenów, nie współdzielonych kont
   ludzi. Tokeny rotuj i przechowuj w menedżerze haseł.
5. Ustaw sesję Access krótszą niż sesję aplikacji; wymuszaj ponowne
   logowanie po zmianie urządzenia.
6. Włącz logi Access i przeglądaj je przy incydentach.
7. Nie traktuj Access jako zamiennika logowania aplikacji. Aplikacja
   nadal wymaga własnego konta i hasła oraz sprawdza role.
8. Zablokuj bezpośredni dostęp do origin (firewall, brak publicznego
   portu). Jeśli origin jest osiągalny z pominięciem Cloudflare, Access
   nic nie chroni.

### 9.5. Uwagi o prywatności

Tunnel i Access oznaczają, że metadane połączeń (adresy IP, czasy,
ścieżki) przechodzą przez Cloudflare. Uwzględnij to w polityce
prywatności szkoły. Nie wysyłaj przez tunel danych, których szkoła nie
może przetwarzać w chmurze. Aplikacja nadal nie przechowuje danych
osobowych uczniów.

### 9.6. Studenci

Dla panelu ucznia możesz:

- pozostawić dostęp publiczny z WAF i rate limitingiem (bez Access),
- albo objąć go Access/SSO, jeśli szkoła ma taki mechanizm.

Nigdy nie udostępniaj panelu ucznia bez TLS. Loginy i hasła nie mogą
podróżować po HTTP.

---

## 10. Rozwiązywanie problemów i FAQ

### 10.1. Kamera nie działa

**Objaw:** brak przycisku kamery lub komunikat o HTTPS.

**Przyczyna:** przeglądarka wymaga bezpiecznego kontekstu (HTTPS albo
`localhost`). Adres `http://<ADRES-SERWERA>:3000` na tablecie nim nie
jest.

**Rozwiązanie:**

1. Użyj czytnika USB/Bluetooth albo wpisz kod ręcznie.
2. Wdróż HTTPS w sieci lokalnej (certyfikat wewnętrzny) albo tryb
   `tunnel`.
3. Sprawdź, czy przeglądarka obsługuje `BarcodeDetector`; jeśli nie,
   pozostaje czytnik USB lub nowsza przeglądarka.

### 10.2. Zapomniane hasło

Administrator resetuje hasło w panelu **Konta → Reset hasła**. Nowe hasło
jest widoczne jednorazowo; uczeń zmienia je przy logowaniu. Nie ma
odzyskiwania przez e-mail.

### 10.3. Konto zablokowane

- Po 10 błędnych logowaniach konto jest blokowane na 15 minut.
- Po 8 błędnych kodach odbioru konto personelu jest blokowane na 15 minut.
- Po 300 błędnych logowaniach z jednego IP blokowane jest IP.

Odczekaj czas z komunikatu. Jeśli blokada jest fałszywa (np. wspólny IP
szkoły), administrator może zwiększyć limity w `config.json` i
zrestartować usługę. Nie wyłączaj limiterów bez powodu.

### 10.4. Pełny dysk

Objawy: baza nie zapisuje, backup się nie tworzy, usługa nie startuje.

```bash
df -h /var/lib/zsl-order
du -sh /var/lib/zsl-order/backups
sudo journalctl -u zsl-order -n 100 --no-pager
```

Usuń najstarsze kopie zgodnie z polityką (nigdy wszystkich), zwolnij
miejsce na logi journald i sprawdź `SystemMaxUse`. Nie usuwaj
`baza.db-wal` ani `baza.db-shm` przy działającej usłudze.

### 10.5. Usługa nie startuje

```bash
sudo systemctl status zsl-order
sudo journalctl -u zsl-order -n 100 --no-pager
```

Najczęstsze przyczyny:

- brak `EnvironmentFile` lub błędny `BIND_ADDR`,
- port zajęty (`ss -ltnp | grep 3000`),
- brak prawa zapisu do `/var/lib/zsl-order`,
- brak `frontend/` względem katalogu roboczego,
- `jwt.key` należy do innego użytkownika,
- w trybie `tunnel` `BIND_ADDR` nie jest pętlą lub brak `PUBLIC_URL`.

### 10.6. 502 / brak odpowiedzi za proxy

- Sprawdź, czy aplikacja nasłuchuje: `curl -v http://127.0.0.1:3000/api/healthz`.
- Sprawdź `proxy_pass` i `Host` w Nginx/Caddy.
- W trybie `tunnel` sprawdź `systemctl status cloudflared` i logi tunelu.
- Upewnij się, że firewall nie blokuje połączenia proxy → aplikacja.

### 10.7. 403 przy zapisie (CSRF/Origin)

- Odśwież stronę i zaloguj się ponownie.
- Upewnij się, że adres w przeglądarce jest zgodny z `PUBLIC_URL` i
  `Host`/`Origin`.
- Nie otwieraj aplikacji jednocześnie pod IP i nazwą domeny — wybierz
  jeden adres.

### 10.8. QR nie działa

- Kod działa tylko dla dzisiejszego zamówienia `ready`.
- Po wydaniu jest jednorazowo zużyty.
- Sprawdź, czy nie minął termin i czy zamówienie nie jest nieodebrane.
- Przy `429` odczekaj blokadę limitera.
- Upewnij się, że tablet ma poprawną datę i strefę czasu.

### 10.9. Backup się nie tworzy

```bash
sudo systemctl status zsl-order-backup.service
sudo journalctl -u zsl-order-backup.service -n 50 --no-pager
df -h /var/lib/zsl-order
```

Sprawdź uprawnienia katalogu `backups/` (`0700`, właściciel
`zsl-order`) i wolne miejsce. Ręcznie uruchom usługę i zweryfikuj kopię.

### 10.10. Nie ma `admin_start.txt`

Plik powstaje tylko przy pierwszym starcie, gdy nie ma żadnego admina.
Jeśli został usunięty przed zmianą hasła, a nie znasz hasła, odtwórz
dostęp zgodnie z procedurą awaryjną w `docs/DEPLOYMENT.md` (rozdział 13).
Nie usuwaj bazy ani nie dodawaj kont ręcznie bez kopii.

### 10.11. Zły czas / zamówienie z innego dnia

Sprawdź strefę czasu i NTP:

```bash
timedatectl
date
```

Aplikacja używa `Europe/Warsaw`. Błędny zegar może powodować, że
zamówienia trafią do złej doby, a QR zostanie odrzucony jako „inny
dzień”.

### 10.12. Baza zablokowana (`database is locked`)

SQLite w WAL powinien obsłużyć współbieżność. Jeśli błędy występują:

- sprawdź, czy nie uruchomiono dwóch instancji na tej samej bazie,
- zwiększ `max_connections` rozsądnie (nie przesadzaj),
- sprawdź miejsce na dysku i uprawnienia plików WAL,
- nie kopiuj bazy przez `cp` przy działającej usłudze — użyj `backup`.

---

## 11. Checklista bezpieczeństwa przed produkcją

Wypełnij przed pierwszym dniem pracy. Każdy punkt powinien być
potwierdzony przez osobę odpowiedzialną.

### 11.1. Dostęp i konta

- [ ] Konto admina ma unikalne, długie hasło w menedżerze haseł.
- [ ] `admin_start.txt` został usunięty po pierwszym logowaniu.
- [ ] Konto `admin` nie jest używane do codziennej pracy kuchni.
- [ ] Konta uczniów i kuchni mają wymuszoną zmianę hasła.
- [ ] Konta nieaktywne i testowe są zdezaktywowane lub usunięte.
- [ ] Administrator nie może odebrać uprawnień własnemu kontu (test).

### 11.2. Sieć i TLS

- [ ] Ruch do paneli odbywa się po HTTPS albo przez `localhost`.
- [ ] W trybie `local` firewall ogranicza port do sieci szkolnej.
- [ ] W trybie `tunnel` port aplikacji nie jest publiczny.
- [ ] `PUBLIC_URL`, `Host` i `Origin` są spójne.
- [ ] Cloudflare Access obejmuje `/admin`, `/kitchen` i `/api/staff`,
      `/api/admin`.
- [ ] MFA jest włączone dla kont administracyjnych.
- [ ] HSTS jest aktywne w trybie HTTPS/tunnel.

### 11.3. Pliki i uprawnienia

- [ ] `/var/lib/zsl-order` ma `0700`, właściciel `zsl-order`.
- [ ] `baza.db`, `jwt.key`, `admin_start.txt` mają `0600`.
- [ ] `/etc/zsl-order/zsl-order.env` ma `0640`, `root:zsl-order`.
- [ ] `/opt/zsl-order` i `frontend/` są tylko do odczytu dla usługi.
- [ ] W repozytorium nie ma `.env`, `jwt.key`, `*.db`, `admin_start.txt`
      ani kopii zapasowych.

### 11.4. Kopie i retencja

- [ ] `zsl-order-backup.timer` jest aktywny i widoczny w
      `systemctl list-timers`.
- [ ] Ręczny backup kończy się sukcesem, a `--verify` przechodzi.
- [ ] Kopia jest wysyłana poza serwer i szyfrowana.
- [ ] Odtworzenie z kopii zostało przetestowane na izolowanym hoście.
- [ ] Retencja zamówień i audytu jest ustalona i zapisana w polityce.
- [ ] Zespół wie, że kopie zawierają dane starsze niż retencja.

### 11.5. Limity i monitoring

- [ ] Limity logowania i odbioru są włączone.
- [ ] `max_uncollected` jest ustawione zgodnie z polityką szkoły.
- [ ] Monitoring sprawdza `/api/healthz` i wiek najnowszej kopii.
- [ ] Alert o nieudanym backupie trafia do człowieka, który go czyta.
- [ ] Dysk ma monitorowany próg zapełnienia (np. 80%).
- [ ] Logi journald mają limit rozmiaru i retencji.

### 11.6. Procesy i dokumentacja

- [ ] Instrukcja jest wydrukowana i dostępna w kuchni.
- [ ] Ściągawki dla ucznia i kuchni są rozdane.
- [ ] Znany jest sposób zgłaszania incydentów i kontakt do administratora.
- [ ] Aktualizacje są planowane poza godzinami pracy stołówki.
- [ ] Poprzednia binarka (`zsl-order.prev`) jest zachowana do rollbacku.
- [ ] Osoba odpowiedzialna przetestowała logowanie, zamówienie, QR,
      wydanie, raport i backup.

### 11.7. Podpisy

| Obszar | Osoba | Data | Podpis |
| --- | --- | --- | --- |
| Dostęp i konta | | | |
| Sieć i TLS | | | |
| Kopie i retencja | | | |
| Monitoring | | | |
| Szkolenie personelu | | | |

> Checklista nie zastępuje testów. Przed produkcją wykonaj pełny
> `scripts/smoke-*.sh` na izolowanej bazie i test odtworzenia kopii.
