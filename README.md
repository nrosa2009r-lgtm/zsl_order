# ZSL Order

System zamawiania posiłków dla stołówki szkolnej ZSŁ. Trzy niezależne panele
(uczeń, kuchnia, administrator), kody QR do odbioru zamówień i projekt
nastawiony na minimalizację danych: **system nie przechowuje imion, nazwisk,
adresów e-mail, numerów telefonów ani innych danych osobowych uczniów**.

Uczniowie otrzymują wyłącznie losowe loginy i hasła. W systemie występują pod
pseudonimowym kodem (`display_code`). Płatność odbywa się przy ladzie
(gotówka lub karta), więc aplikacja nie dotyka danych kart płatniczych.

> Dokumentacja techniczna: [API](docs/API.md) ·
> [architektura](docs/ARCHITECTURE.md) ·
> [bezpieczeństwo](docs/SECURITY.md) ·
> [wdrożenie](docs/DEPLOYMENT.md) ·
> [baza danych](docs/DATABASE.md) ·
> [prywatność i RODO](docs/PRIVACY.md)

---

## Spis treści

- [Najważniejsze cechy](#najważniejsze-cechy)
- [Role i przepływy](#role-i-przepływy)
- [Stos technologiczny](#stos-technologiczny)
- [Architektura w skrócie](#architektura-w-skrocie)
- [Szybki start](#szybki-start)
- [Pierwsze uruchomienie i konta](#pierwsze-uruchomienie-i-konta)
- [Panele](#panele)
- [Cykl życia zamówienia](#cykl-życia-zamówienia)
- [Konfiguracja](#konfiguracja)
- [Bezpieczeństwo w skrócie](#bezpieczeństwo-w-skrócie)
- [Prywatność i RODO](#prywatność-i-rodo)
- [Testy](#testy)
- [Wdrożenie produkcyjne](#wdrożenie-produkcyjne)
- [Struktura projektu](#struktura-projektu)
- [Dokumentacja](#dokumentacja)
- [Ograniczenia i dalszy rozwój](#ograniczenia-i-dalszy-rozwój)
- [Licencja](#licencja)

---

## Najważniejsze cechy

- **Trzy panele w jednej aplikacji**: uczeń, kuchnia, administrator.
- **Zero rejestracji i zero danych osobowych**: konta zakłada administrator,
  loginy i hasła są generowane losowo.
- **Kody QR do odbioru**: każde zamówienie ma krótki kod (`K7M2QP`) oraz
  token QR (`ZSL:<48 znaków hex>`). Kuchnia może zeskanować QR kamerą,
  użyć czytnika USB albo wpisać kod ręcznie.
- **Bezpieczne sesje**: tokeny 256-bit, w bazie tylko skrót SHA-256,
  ciasteczka `HttpOnly` + `SameSite=Strict`, opcjonalnie `Secure`.
- **Ochrona CSRF**: token per sesja + nagłówek `X-CSRF-Token` + kontrola
  pochodzenia żądania i `Sec-Fetch-Site`.
- **RBAC**: student, kitchen, admin; każdy endpoint ma jawny guard roli.
- **Hasła Argon2id** z losową solą; wymuszona zmiana hasła startowego.
- **Silne nagłówki bezpieczeństwa**: restrykcyjny CSP bez `inline`,
  `frame-ancestors 'none'`, `nosniff`, `no-referrer`, COOP/CORP, HSTS
  (po włączeniu HTTPS).
- **Walidacja wejścia i parametryzowany SQL** (`sqlx`), limit ciała żądania
  64 KiB, limit prób logowania.
- **Dziennik zdarzeń** (audyt) z automatycznym czyszczeniem po 90 dniach
  oraz techniczne logi dzienne w `logs/` (bez haseł, tokenów i adresów IP).
- **Statystyki i wykresy aktywności**: zamówienia dziennie, utarg, rozkład
  godzinowy, statusy i najczęściej zamawiane pozycje — rysowane na canvasie
  bez zewnętrznych bibliotek.
- **Eksport CSV metadanych kont** (bez haseł) i **twarde usuwanie kont**
  wraz z historią zamówień i sesjami.
- **Frontend bez build-stepu i bez CDN**: czysty HTML/CSS/JS, wszystko
  serwowane z tej samej domeny.
- **SQLite + WAL**: prosta, szybka i łatwa w backupie baza dla jednej
  stołówki; brak zewnętrznego serwera bazy danych.

## Role i przepływy

| Rola | Panel | Uprawnienia |
| --- | --- | --- |
| `student` | `/student` | przegląd menu, składanie zamówień, anulowanie własnego zamówienia w statusie `new`, podgląd własnych zamówień i QR, zmiana własnego hasła |
| `kitchen` | `/kitchen` | przegląd zamówień, wyszukiwanie po kodzie/QR, zmiana statusu `preparing → ready → completed`, oznaczanie płatności jako opłaconej, zarządzanie menu |
| `admin` | `/admin` | wszystko powyższe + statystyki, konta, reset haseł, ustawienia, audyt, pełne zarządzanie zamówieniami i menu |

Nie ma publicznej rejestracji. Uczeń nie podaje żadnych danych — dostaje
wydrukowany/wręczony login i hasło od administratora lub wychowawcy.

## Stos technologiczny

| Warstwa | Technologia | Uwagi |
| --- | --- | --- |
| Język | Rust 2024 edition | wymaga Rust 1.85+ |
| HTTP | Axum 0.7 / Tokio 1 | lekki, asynchroniczny serwer |
| Baza | SQLite (sqlx 0.8) | WAL, klucze obce, transakcje |
| Hasła | Argon2id (`argon2` 0.5) | losowa sól, domyślne parametry |
| Tokeny | `rand` 0.9 + `sha2` + `hex` | sesje, CSRF, QR |
| QR | `qrcode` 0.14 (SVG) | generowany po stronie serwera |
| Czas | `chrono` | UTC w bazie, dzień lokalny w statystykach |
| Frontend | HTML + CSS + vanilla JS | bez frameworka, bez CDN, bez build-stepu |
| Statyczne pliki | `tower-http::ServeDir` | tylko z katalogu `frontend/` |

## Architektura w skrócie

```text
Przeglądarka
   │  HTTPS (zalecane) / HTTP (tylko lokalnie)
   ▼
Axum Router
   ├─ nagłówki bezpieczeństwa (CSP, HSTS, COOP/CORP, …)
   ├─ origin_guard (Sec-Fetch-Site / Origin)
   ├─ limit ciała 64 KiB
   ├─ /api/auth/login, /api/public/info        (publiczne)
   └─ middleware sesji + CSRF                  (chronione)
        ├─ /api/auth/*                        (każda zalogowana rola)
        ├─ /api/student/*                     (student)
        ├─ /api/staff/*                       (kitchen, admin)
        └─ /api/admin/*                       (admin)
   ▼
Handlery → sqlx → SQLite (baza.db, WAL)
   ▲
   └─ frontend/ (HTML, CSS, JS) serwowany przez ServeDir
```

Szczegóły warstw, kolejności middleware i przepływów opisuje
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Szybki start

### Wymagania

- Rust 1.85+ i Cargo
- Linux, macOS lub Windows (produkcyjnie zalecany Linux)
- brak zewnętrznej bazy danych — SQLite jest wbudowany

### Uruchomienie lokalne

```bash
git clone https://github.com/nrosa2009r-lgtm/zsl_order.git
cd zsl_order

# opcjonalnie: bezpieczna konfiguracja lokalna (tylko 127.0.0.1)
cp config.example.json config.json

cargo run
```

Aplikacja wypisze adres, np. `http://127.0.0.1:3000`, oraz **jednorazowe
dane startowego administratora**. Te same dane trafią do pliku
`admin_start.txt` z uprawnieniami `0600`.

```text
==============================================
ZSL ORDER - STARTOWE KONTO ADMINISTRATORA
==============================================
Login:  admin-xxxxxxxxxx
Hasło:  XXXXXXXXXXXXXXXX
...
```

Po pierwszym zalogowaniu system wymusi zmianę hasła, a plik
`admin_start.txt` zostanie automatycznie usunięty.

### Pierwsze kroki

1. Zaloguj się do `/admin` danymi z konsoli/pliku.
2. Zmień hasło startowe (system wymusi to przed dostępem do panelu).
3. W zakładce **Konta** wygeneruj konta uczniów i kuchni.
4. **Zapisz jednorazowo wyświetlone hasła** — nie są przechowywane w
   postaci jawnej. Późniejszy eksport CSV zawiera tylko metadane kont
   (bez haseł); hasła można zapisać wyłącznie bezpośrednio po wygenerowaniu
   kont lub po resecie hasła.
5. Przekaż uczniom loginy i hasła w bezpieczny sposób (np. wydruk,
   koperta, szkolny dziennik elektroniczny — poza systemem).
6. W zakładce **Menu** uzupełnij ofertę, a w **Ustawieniach** nazwę
   stołówki, informację o płatności i limity zamówień.

## Pierwsze uruchomienie i konta

- Startowe konto administratora powstaje **tylko wtedy**, gdy w bazie nie ma
  żadnego konta z rolą `admin`.
- Login i hasło są losowe (hasło 18 znaków, gwarantowana litera i cyfra).
- Plik `admin_start.txt` ma uprawnienia `0600` (tylko właściciel) i jest
  usuwany po zmianie hasła przez administratora.
- Nowe konta (`student`, `kitchen`, `admin`) generuje administrator.
  Hasła są zwracane **tylko raz** w odpowiedzi API i nigdy nie są zapisywane
  jawnie.
- Każde nowe konto ma `must_change_password = 1`; do momentu zmiany hasła
  działa tylko `/api/auth/me`, `/api/auth/logout` i
  `/api/auth/change-password`.
- Administrator może resetować hasła (również jednorazowo) i dezaktywować
  konta. Dezaktywacja oraz zmiana roli unieważniają wszystkie sesje konta.
- Administrator nie może odebrać uprawnień własnemu kontu (ochrona przed
  przypadkowym zablokowaniem się).

## Panele

### Panel ucznia — `/student`

- menu pogrupowane według kategorii, ceny w PLN, alergeny, dostępność,
- koszyk z ilościami (1–10 szt. na pozycję),
- wybór płatności przy odbiorze: gotówka lub karta,
- opcjonalna godzina odbioru (`HH:MM`),
- potwierdzenie zamówienia z kodem odbioru i kodem QR,
- lista własnych zamówień, statusy, anulowanie tylko świeżego zamówienia,
- zmiana własnego hasła.

### Panel kuchni — `/kitchen`

- zamówienia otwarte (`new`, `preparing`, `ready`), dzisiejsze lub wszystkie,
- karty zamówień z pozycjami, kwotą, sposobem płatności i godziną odbioru,
- wyszukiwanie po krótkim kodzie lub tokenie QR,
- skaner QR kamery (`BarcodeDetector`, jeśli wspierany) + ręczne pole
  (działa też z czytnikiem USB działającym jak klawiatura),
- zmiana statusu: przyjęte → w przygotowaniu → gotowe → wydane,
- oznaczenie płatności jako opłaconej,
- zarządzanie menu (dodawanie, edycja, ukrywanie pozycji).

### Panel administratora — `/admin`

- statystyki: konta, zamówienia dzisiaj, utarg dzisiejszy, otwarte
  zamówienia, oczekujące płatności, dostępne pozycje menu, aktywne sesje,
- wykresy aktywności za 7/14/30/90 dni: liczba zamówień, utarg dzienny,
  rozkład godzinowy, statusy i najczęściej zamawiane pozycje,
- generowanie kont (1–100 jednorazowo) z jednorazowymi hasłami,
- lista kont z filtrami (rola, aktywność, szukanie po loginie/kodzie),
- aktywacja/dezaktywacja, zmiana roli, reset hasła, **twarde usunięcie
  konta** (wraz z zamówieniami i sesjami; z zabezpieczeniem przed usunięciem
  siebie i ostatniego aktywnego administratora),
- eksport CSV **metadanych** kont (bez haseł; hasła tylko jednorazowo po
  wygenerowaniu lub resecie),
- pełne zarządzanie menu,
- wszystkie zamówienia i ręczna korekta statusów/płatności,
- ustawienia stołówki i limitów,
- dziennik audytu (ostatnie 200 wpisów, maks. 500).

## Cykl życia zamówienia

```text
                 ┌──────────────┐
                 │     new      │  (uczeń może anulować)
                 └──────┬───────┘
                        │ kuchnia przyjmuje
                        ▼
                 ┌──────────────┐
                 │  preparing   │
                 └──────┬───────┘
                        │ kuchnia kończy przygotowanie
                        ▼
                 ┌──────────────┐
                 │    ready     │  (gotowe do odbioru)
                 └──────┬───────┘
                        │ wydanie
                        ▼
                 ┌──────────────┐
                 │  completed   │  płatność automatycznie = paid
                 └──────────────┘

      anulowane w dowolnym momencie przez admina:
                 new / preparing / ready → cancelled
```

Płatność: `unpaid` → `paid`. Kuchnia może tylko oznaczyć `paid`; admin może
ustawić oba stany. Wydanie zamówienia automatycznie ustawia `paid`.

## Konfiguracja

Plik `config.json` jest tworzony automatycznie przy pierwszym uruchomieniu
(jeśli go nie ma). W repozytorium znajduje się `config.example.json`.
Nie zawiera sekretów.

| Pole | Domyślnie | Zakres / znaczenie |
| --- | --- | --- |
| `ip` | `0.0.0.0` | adres nasłuchu; produkcyjnie zalecane `127.0.0.1` za reverse proxy |
| `port` | `3000` | port HTTP |
| `max_connections` | `16` | 1–128, rozmiar puli SQLite |
| `db_url` | `sqlite://baza.db` | ścieżka/URL bazy; akceptowane też `baza.db` |
| `session_hours` | `12` | 1–720, czas życia sesji |
| `secure_cookies` | `false` | **ustaw `true` przy HTTPS**; włącza `Secure`, HSTS i `upgrade-insecure-requests` w CSP |

Uwagi:

- `max_connections` większy niż potrzeby nie przyspiesza SQLite — to baza
  jednego pliku; 8–16 wystarcza dla stołówki.
- Po zmianie `secure_cookies` na `true` logowanie musi odbywać się przez
  HTTPS, inaczej przeglądarka nie wyśle ciasteczka.
- Konfiguracja jest czytana odpornie: brakujące lub nieprawidłowe pola
  dostają bezpieczne wartości domyślne, a plik jest normalizowany.

## Bezpieczeństwo w skrócie

- Hasła: **Argon2id**, losowa sól, domyślne parametry (19 MiB, 2 iteracje).
- Sesje: 32 losowe bajty → 64 znaki hex; w bazie tylko SHA-256.
- Ciasteczko: `HttpOnly`, `SameSite=Strict`, `Path=/`, opcjonalnie `Secure`.
- CSRF: token 32 bajty na sesję, wymagany w `X-CSRF-Token` dla metod
  zmieniających stan; porównanie w czasie stałym.
- Dodatkowo: blokada `Sec-Fetch-Site: cross-site` i niezgodnego `Origin`.
- RBAC na każdym endpoincie; brak dostępu po ID do cudzych zamówień.
- Rate limiting logowania: 10 błędów na konto / 80 na IP w 10 min → blokada
  15 min; brak ujawniania, czy login istnieje (wyrównanie czasu dummy hash).
- Walidacja długości i formatów, parametryzowany SQL, limit 64 KiB.
- CSP bez `inline`, `frame-ancestors 'none'`, `nosniff`, `no-referrer`,
  `Permissions-Policy`, COOP/CORP; HSTS po włączeniu HTTPS.
- Audyt zdarzeń bez danych osobowych; 90 dni retencji.
- QR to losowy token 192-bit; krótki kod jest tylko wygodą i wymaga
  zalogowanego pracownika.

Pełna analiza, w tym ograniczenia i checklista wdrożeniowa:
[docs/SECURITY.md](docs/SECURITY.md).

## Prywatność i RODO

Projekt stosuje **minimalizację danych i pseudonimizację**, a nie
„anonimowość” w sensie prawnym. System nie zbiera danych osobowych
uczniów, ale jeśli szkoła przechowuje gdzieś mapowanie losowego loginu na
konkretną osobę, to po stronie szkoły mogą powstać dane osobowe.

- Brak pól: imię, nazwisko, e-mail, telefon, adres, klasa, data urodzenia,
  zdjęcie, dane karty.
- Uczeń = losowy login + pseudonimowy kod (`UCZ-AB12C`).
- Zamówienie zawiera identyfikator techniczny konta, pozycje (nazwa i cena
  w momencie zakupu), kwotę, sposób i status płatności, godzinę odbioru oraz
  znaczniki czasu.
- QR zawiera wyłącznie losowy token — bez loginu i danych ucznia.
- Adresy IP nie są zapisywane w bazie; limiter logowania działa w pamięci.
- Audyt zawiera identyfikator konta, typ zdarzenia i techniczny opis.

Przed wdrożeniem szkoła powinna określić podstawę prawną, okres
przechowywania zamówień, zasady dostępu i ewentualną ocenę skutków dla
ochrony danych (DPIA). Szczegóły i rekomendacje:
[docs/PRIVACY.md](docs/PRIVACY.md).

## Testy

```bash
cargo test          # testy jednostkowe (konfiguracja, hasła, tokeny, walidacja)
cargo clippy        # analiza statyczna
cargo build --release
```

Testy jednostkowe obejmują m.in. roundtrip hasła Argon2, losowość tokenów,
porównanie w czasie stałym, walidację godziny odbioru oraz escapowanie CSV.
Testy end-to-end (API + role + CSRF + QR + statystyki + eksport + usuwanie
kont) wykonuje się na izolowanej kopii bazy; opis
w [docs/DEPLOYMENT.md](docs/DEPLOYMENT.md#11-testy-i-smoke-test).

## Wdrożenie produkcyjne

Minimalny, bezpieczny scenariusz:

1. Zbuduj release: `cargo build --release`.
2. Utwórz dedykowanego użytkownika systemowego i katalog `/opt/zsl_order`.
3. Skopiuj binarkę, `frontend/` i `config.example.json` → `config.json`.
4. Ustaw `ip: 127.0.0.1`, `secure_cookies: true`, uruchom za Nginx/Caddy
   z TLS.
5. Uruchom jako usługa systemd z `UMask=0077`, `NoNewPrivileges=yes`,
   `ProtectSystem=strict`.
6. Odbierz dane startowego admina, zaloguj się, zmień hasło (plik zniknie).
7. Skonfiguruj backup SQLite (`sqlite3 baza.db ".backup ..."`) i monitoring.

Gotowe przykłady systemd i Nginx, backup, aktualizacja i checklista:
[docs/DEPLOYMENT.md](docs/DEPLOYMENT.md).

## Struktura projektu

```text
zsl_order/
├── Cargo.toml
├── Cargo.lock
├── config.example.json
├── README.md
├── docs/
│   ├── API.md
│   ├── ARCHITECTURE.md
│   ├── DATABASE.md
│   ├── DEPLOYMENT.md
│   ├── PRIVACY.md
│   └── SECURITY.md
├── frontend/
│   ├── login.html / login.js
│   ├── student.html / student.js
│   ├── kitchen.html / kitchen.js
│   ├── admin.html / admin.js
│   ├── common.js          # API, CSRF, dialogi, formatowanie
│   ├── staff-common.js    # wspólne UI kuchni i admina
│   ├── charts.js          # wykresy canvas bez zależności
│   └── style.css
└── src/
    ├── main.rs            # router, nagłówki, origin guard, start
    ├── config.rs          # config.json
    ├── state.rs           # AppState
    ├── db.rs              # schemat, seed, audyt, bootstrap admina
    ├── auth.rs            # sesje, CSRF, role, limiter
    ├── security.rs        # Argon2, tokeny, kody
    ├── logging.rs         # tracing + dzienne logi w logs/
    ├── util.rs            # czas, walidacje
    ├── error.rs           # spójne błędy JSON
    └── handlers/
        ├── auth.rs
        ├── public.rs
        ├── menu.rs
        ├── orders.rs
        └── admin.rs
```

Katalog `logs/` powstaje automatycznie przy starcie i nie jest wersjonowany.

## Dokumentacja

| Dokument | Zawartość |
| --- | --- |
| [docs/API.md](docs/API.md) | pełny opis endpointów, modeli, błędów i przykładów `curl` |
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | moduły, middleware, przepływy, decyzje projektowe |
| [docs/SECURITY.md](docs/SECURITY.md) | model zagrożeń, mechanizmy ochrony, ograniczenia, checklista |
| [docs/DEPLOYMENT.md](docs/DEPLOYMENT.md) | systemd, Nginx/TLS, backup, aktualizacje, monitoring |
| [docs/DATABASE.md](docs/DATABASE.md) | schemat SQL, indeksy, ustawienia, zapytania operacyjne |
| [docs/PRIVACY.md](docs/PRIVACY.md) | zakres danych, pseudonimizacja, retencja, RODO |

## Ograniczenia i dalszy rozwój

- SQLite i jedna instancja aplikacji — rozwiązanie dla jednej stołówki;
  nie należy udostępniać pliku bazy wielu instancjom przez NFS.
- Brak płatności online i integracji z kartą — celowo, aby nie dotykać
  danych płatniczych.
- Brak wysyłki e-mail/SMS i brak samoobsługowego resetu hasła — celowo,
  aby nie zbierać danych kontaktowych.
- Limiter logowania działa w pamięci i zeruje się po restarcie; przy
  wdrożeniu za reverse proxy widzi adres proxy (patrz dokumentacja wdrożenia).
- Krótki kod odbioru ma ok. 28 bitów entropii; jest przeznaczony wyłącznie
  dla zalogowanego personelu, a nie jako publiczny sekret.
- Brak wbudowanego harmonogramu retencji zamówień — administrator powinien
  ustalić i okresowo wykonywać politykę usuwania/anonymizacji.
- Brak formalnego systemu migracji — schemat jest idempotentny
  (`CREATE TABLE IF NOT EXISTS`); zmiany wymagają ręcznej migracji.
- Brak MFA dla kont administracyjnych; zalecane silne, unikalne hasła
  i ograniczenie dostępu sieciowego.
- Hasła nie są odzyskiwalne z bazy (tylko Argon2id); eksport CSV zawiera
  wyłącznie metadane kont. Ewentualny plik z hasłami trzeba zabezpieczyć
  i zniszczyć po przekazaniu uczniom.
- Logi techniczne w `logs/` nie mają wbudowanej retencji — administrator
  powinien okresowo je usuwać lub skonfigurować `logrotate`. Nie zawierają
  haseł, tokenów sesji, ciasteczek, ciał żądań ani adresów IP.

## Licencja

Repozytorium nie zawiera obecnie pliku `LICENSE`. Przed publicznym
udostępnieniem kodu dodaj wybraną licencję (np. MIT, Apache-2.0) lub
oznacz projekt jako wewnętrzny. Bez licencji domyślnie obowiązują
wszystkie prawa zastrzeżone.
