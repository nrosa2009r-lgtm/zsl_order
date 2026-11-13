# Architektura — ZSL Order

Dokument opisuje budowę aplikacji, podział odpowiedzialności, kolejność
warstw HTTP, przepływy danych i decyzje projektowe. Szczegóły kontraktu
API znajdują się w [API.md](API.md), a mechanizmy ochrony w
[SECURITY.md](SECURITY.md).

## 1. Cele projektowe

1. **Minimalizacja danych** — brak pól osobowych; uczeń to losowy login
   i pseudonimowy kod.
2. **Prostota operacyjna** — jedna binarka, jedna baza SQLite, brak
   zewnętrznych usług i CDN.
3. **Bezpieczeństwo domyślne** — restrykcyjne ciasteczka, CSRF, RBAC,
   CSP, Argon2id, rate limiting.
4. **Szybkość** — asynchroniczny Rust, lokalna baza, brak narzutu
   frameworka frontendowego.
5. **Czytelność** — małe moduły, jawne trasy i guardy ról.

## 2. Widok ogólny

```text
┌──────────────────────────────────────────────────────────────┐
│ Przeglądarka (HTML/CSS/JS, bez build-stepu)                  │
│  login · student · kitchen · admin · common.js               │
└───────────────────────────┬──────────────────────────────────┘
                            │ HTTP(S), ciasteczko sesji
┌───────────────────────────▼──────────────────────────────────┐
│ Axum Router (src/main.rs)                                    │
│  client_ip → headers → origin_guard → routes                 │
│                                                              │
│  publiczne:  /api/auth/login, /api/public/info, /api/healthz │
│  chronione:  auth_middleware (sesja + CSRF + forced change)  │
│              ├─ /api/auth/*                                  │
│              ├─ /api/student/*  + require_student            │
│              ├─ /api/staff/*    + require_staff              │
│              └─ /api/admin/*    + require_admin              │
└───────────────────────────┬──────────────────────────────────┘
                            │ sqlx
┌───────────────────────────▼──────────────────────────────────┐
│ SQLite (baza.db, WAL)                                        │
│  users · sessions · menu_items · orders · order_items        │
│  settings · audit_log                                        │
└──────────────────────────────────────────────────────────────┘
```

Aplikacja jest monolitem: serwer HTTP, logika biznesowa, dostęp do bazy
i serwowanie statycznych plików znajdują się w jednym procesie. To
świadoma decyzja — dla jednej stołówki prostsza i bezpieczniejsza niż
rozdzielanie usług.

## 3. Moduły

| Plik | Odpowiedzialność |
| --- | --- |
| `src/main.rs` | konfiguracja routera, trasy, nagłówki bezpieczeństwa, `origin_guard`, serwowanie `frontend/`, start serwera |
| `src/config.rs` | wczytywanie `.env` i `config.json`, walidacja trybu wdrożenia, `PUBLIC_URL`, zaufanych proxy i `sqlite_url()` |
| `src/client_ip.rs` | ustalanie adresu klienta: gniazdo TCP w `local`, `CF-Connecting-IP` od zaufanego proxy w `tunnel` |
| `src/guard.rs` | walidacja `Host` i `Origin` dla obu trybów wdrożenia |
| `src/state.rs` | `AppState`: pula DB, konfiguracja, dummy hash, limiter logowania, sekret JWT |
| `src/db.rs` | schemat, indeksy, seed ustawień i menu, czyszczenie, bootstrap admina, audyt, ustawienia |
| `src/auth.rs` | sesje cookie i Bearer, JWT (claims, podpis, weryfikacja), CSRF, guardy ról, limiter logowania |
| `src/security.rs` | Argon2id, generatory loginów/haseł/kodów, SHA-256 tokenów, sekret `jwt.key`, porównanie w czasie stałym |
| `src/logging.rs` | inicjalizacja `tracing` (JSON na stdout, journald), middleware logujące żądania bez danych wrażliwych |
| `src/util.rs` | czas UTC, doba i godziny w `Europe/Warsaw`, formatowanie dat, walidacja `HH:MM` |
| `src/error.rs` | `AppError`, spójne odpowiedzi JSON, mapowanie błędów SQL/Argon2/IO, logowanie błędów wewnętrznych |
| `src/handlers/auth.rs` | login, wydanie tokenu JWT, logout, `me`, zmiana hasła, logowanie zdarzeń bezpieczeństwa |
| `src/handlers/public.rs` | publiczne informacje o stołówce i `/api/healthz` |
| `src/handlers/menu.rs` | menu ucznia i personelu, CRUD pozycji |
| `src/handlers/orders.rs` | składanie/anulowanie zamówień, QR, kolejka kuchni, pickup, zmiana statusów |
| `src/handlers/admin.rs` | statystyki, aktywność, konta, generowanie, reset haseł, usuwanie kont, eksport CSV, ustawienia, audyt |

## 4. Cykl żądania HTTP

Kolejność warstw (od zewnątrz):

0. **`client_ip`** — ustala adres klienta: w `local` zawsze z gniazda
   TCP, w `tunnel` z `CF-Connecting-IP` tylko od zaufanego proxy.
   Wynik trafia do rozszerzeń żądania jako `ClientIp`.
1. **`request_log`** — mierzy czas i loguje metodę, ścieżkę bez query,
   status i czas odpowiedzi. Nie loguje nagłówków, ciasteczek, tokenów
   CSRF, ciał żądań ani adresów IP. Dla `/api/*` poziom zależy od statusu
   (info / warn / error), dla statyków sukces jest na poziomie `debug`.
2. **`security_headers`** — dodaje CSP, `X-Content-Type-Options`,
   `X-Frame-Options`, `Referrer-Policy`, `Permissions-Policy`,
   COOP/CORP; HSTS tylko w `tunnel`; dla `/api/*` dodaje
   `Cache-Control: no-store`.
3. **`origin_guard`** — waliduje `Host` (w `tunnel` musi wskazywać
   `PUBLIC_URL` albo loopback), a dla metod innych niż
   `GET`/`HEAD`/`OPTIONS` blokuje `Sec-Fetch-Site: cross-site` oraz
   niezgodny `Origin`.
4. **`RequestBodyLimitLayer`** — limit 64 KiB.
5. **Router** — dopasowanie ścieżki i metody; statyczne pliki z
   `frontend/` przez `ServeDir`, `/` przekierowuje na `/login`.
6. **`auth_middleware`** (tylko trasy chronione) — dwa warianty:
   - **ciasteczko `zsl_session`**:
     - odczyt ciasteczka i walidacja formatu (64 hex),
     - SHA-256 tokenu i odczyt sesji + użytkownika z bazy,
     - sprawdzenie wygaśnięcia i `active`,
     - weryfikacja CSRF dla metod zmieniających stan,
     - odświeżenie `last_seen_at` maks. raz na 5 minut,
   - **`Authorization: Bearer <JWT>`**:
     - weryfikacja HS256, `iss`, `aud`, `exp` i rozmiaru tokenu,
     - `jti` → `token_hash = SHA-256("jwt:" + jti)` → wiersz `sessions`,
     - zgodność `sub`/`login`/`role`/`code` z aktualnym kontem,
     - sprawdzenie `active` i wygaśnięcia; brak CSRF (przeglądarka nie
       dołącza nagłówka automatycznie),
   - wspólna blokada kont z `must_change_password`,
   - wstawienie `AuthUser` do rozszerzeń żądania.
7. **Guard roli** (`require_student`, `require_staff`, `require_admin`) —
   porównanie `AuthUser.role`.
8. **Handler** — walidacja, logika, SQL, audyt, odpowiedź JSON.

Dzięki `auth_middleware` handlery nie muszą same sprawdzać sesji ani CSRF.
Guardy ról są nakładane przez `route_layer` na poddrzewa tras, więc nowy
endpoint w `/api/admin` automatycznie dziedziczy wymóg roli `admin`.

## 5. Przepływy

### 5.1. Logowanie (sesja ciasteczkowa)

```text
POST /api/auth/login
  → limiter.check(ip, login)
  → SELECT user BY login
      ├─ brak użytkownika → verify(dummy_hash) → 401 (wyrównanie czasu)
      └─ jest → verify(Argon2id)
  → nieaktywne / błędne hasło → 401
  → wygeneruj token sesji (32 B) + CSRF (32 B)
  → zapisz SHA-256(token) w sessions
  → Set-Cookie + JSON {user, csrf_token}
```

### 5.2. Token API (JWT)

```text
POST /api/auth/token
  → ta sama weryfikacja hasła i limiter co login
  → wygeneruj jti (16 B) + claims (sub, login, role, code, iss, aud, iat, exp)
  → podpisz HS256 sekretem z jwt.key
  → zapisz SHA-256("jwt:" + jti) w sessions (csrf_token = '', TTL 15 min)
  → audyt token_issued + JSON {access_token, token_type, expires_in, user}

Żądanie chronione:
Authorization: Bearer <JWT>
  → auth_middleware: weryfikacja podpisu/iss/aud/exp
  → odczyt sesji po SHA-256("jwt:" + jti)
  → zgodność claims z aktualnym kontem + active
  → bez CSRF (nagłówek nie jest wysyłany automatycznie)
```

Dzięki wpisowi w `sessions` token JWT można unieważnić natychmiast —
tak samo jak sesję ciasteczkową. `jwt.key` jest wczytywany przy starcie;
jego usunięcie i restart unieważnia wszystkie tokeny.

### 5.3. Złożenie zamówienia

```text
POST /api/student/orders
  → auth + CSRF + rola student
  → sprawdź ordering_enabled, limity otwartych zamówień
  → waliduj płatność, godzinę, pozycje (1–10 szt., bez duplikatów)
  → SELECT menu_items WHERE id IN (...) (QueryBuilder, bindowane)
  → sprawdź istnienie i available
  → policz total_cents po stronie serwera (cena z bazy)
  → BEGIN
      → wygeneruj unikalny code (6 liter) i qr_token (24 B)
      → INSERT orders
      → INSERT order_items (snapshot nazwy i ceny)
    COMMIT
  → audit(order_created)
  → 201 OrderView
```

Transakcja gwarantuje, że zamówienie nie powstanie bez pozycji. Kody są
losowane z ponawianiem przy kolizji (unikalne indeksy w bazie).

### 5.4. QR i odbiór

- QR generuje serwer w `GET /api/student/orders/:id/qr.svg` na podstawie
  `qr_token`; w obrazie jest wyłącznie nieprzejrzysty token, bez loginu,
  kodu ucznia, ID konta, ceny, URL i tokenu sesji.
- Kuchnia skanuje QR (`BarcodeDetector` + `getUserMedia`, z lokalnym
  jsQR gdy `BarcodeDetector` jest niedostępny) albo wpisuje
  kod lub używa czytnika USB działającego jak klawiatura (Enter =
  szukaj). `POST /api/staff/pickup` normalizuje krótki kod do wielkich
  liter, opcjonalnie usuwa starszy prefiks `ZSL:` i szuka po `qr_token`
  lub `code`.
- Pickup akceptuje tylko zamówienie z dzisiejszego dnia w statusie
  `ready`; po `completed` kod jest jednorazowo zużyty. Pickup **nie
  zmienia** statusu — wydanie następuje przez
  `PATCH /api/staff/orders/:id`.
- Każde udane wyszukanie trafia do audytu; nieudane próby zasilają
  `PickupLimiter`.

### 5.5. Zmiana statusu i płatności

- Kuchnia: `new → preparing → ready → completed` (lub pominięcie stanów),
  `completed` automatycznie ustawia `paid`.
- Kuchnia nie może modyfikować zamówień `completed`/`cancelled`.
- Student może anulować tylko własne `new`.
- Administrator może ustawić dowolny dozwolony status i płatność.

### 5.6. Statystyki aktywności

`GET /api/admin/stats/activity?days=1..90`:

```text
→ wybierz zakres [początek doby Europe/Warsaw (dziś - days + 1), teraz]
→ jedno zapytanie o zamówienia z zakresu (created_at, status, payment_status, total)
→ agregacja w Rust:
    ├─ daily[] — liczba, ukończone, anulowane, utarg (tylko paid i nie-cancelled)
    ├─ hourly_today[] — 24 kubełki dla dzisiejszych zamówień
    ├─ status_counts / payment_counts
    └─ totals (orders, completed, cancelled, revenue, średnia)
→ drugie zapytanie: TOP 8 pozycji (JOIN order_items, bez anulowanych)
→ JSON gotowy do wykresów canvas
```

Dni są grupowane według **Europe/Warsaw** (niezależnie od strefy serwera),
a znaczniki czasu w bazie pozostają w UTC. Statystyki nie zawierają danych
osobowych.

### 5.7. Twarde usuwanie konta

`DELETE /api/admin/users/:id`:

```text
→ odrzuć usunięcie własnego konta
→ odczytaj login i rolę celu
→ jeśli admin: sprawdź, czy istnieje inny aktywny administrator
→ BEGIN
    → DELETE order_items dla zamówień użytkownika
    → DELETE orders użytkownika
    → DELETE sessions użytkownika
    → DELETE users (0 wierszy → rollback)
  COMMIT
→ audit(user_deleted) + tracing::warn
```

Operacja jest nieodwracalna i celowo usuwa także historię zamówień oraz
sesje, aby nie pozostawiać osieroconych danych powiązanych z kontem.

### 5.8. Eksport CSV metadanych

`GET /api/admin/users/export.csv` używa tych samych filtrów co lista kont
(`role`, `active`, `q`), limit 20 000 wierszy:

```text
→ QueryBuilder z bindowanymi parametrami
→ nagłówek: login;rola;kod;aktywne;haslo_startowe;utworzono;ostatnie_logowanie
→ każda komórka w cudzysłowach, escapowanie ", ochrona przed formułami (= + - @)
→ BOM UTF-8 + CRLF (Excel)
→ Content-Disposition: attachment; filename="konta-zsl-order-RRRR-MM-DD.csv"
→ Cache-Control: no-store + audit(users_exported)
```

Kolumna `haslo_startowe` mówi tylko, czy konto nadal ma wymuszoną zmianę
hasła — **nie zawiera hasła**. System nie przechowuje haseł jawnych.

### 5.9. Logowanie techniczne

- `logging::init()` konfiguruje `tracing` do zapisu **JSON na stdout**.
  Aplikacja nie tworzy plików ani katalogu `logs/`; pod systemd stdout
  trafia do journald.
- Domyślny filtr: `info,sqlx=warn,hyper=warn,axum=warn`; można nadpisać
  przez `RUST_LOG`.
- Każdy wpis ma `timestamp`, `level` i `fields` (m.in. `event`, `method`,
  `path`, `status`, `latency_ms`).
- Zdarzenia: start/konfiguracja/baza/serwer, logowania (sukces,
  `login_failed`, `login_locked`), wydanie tokenu JWT, zmiany haseł,
  generowanie/aktualizacja/usuwanie kont, reset hasła, eksport CSV, błędy
  wewnętrzne i błędy zapisu audytu.
- **Nigdy** nie logujemy haseł, tokenów sesji/CSRF/JWT, sekretu `jwt.key`,
  ciasteczek ani ciał żądań. Logowanie żądań nie zapisuje parametrów
  zapytania. Adres IP występuje wyłącznie w zdarzeniach bezpieczeństwa
  (`login_failed`, `login_locked`, `pickup_locked`), a zamiast loginu
  zapisywane jest `account_id`.

## 6. Model danych (skrót)

```text
users 1───* sessions
users 1───* orders 1───* order_items
menu_items 1───* order_items (opcjonalnie, ON DELETE SET NULL)
users 1───* audit_log (actor_id, opcjonalnie)
settings (key/value)
```

Pełny schemat: [DATABASE.md](DATABASE.md).

Kluczowe decyzje:

- **Snapshot pozycji** — `order_items.name` i `price_cents` są kopiowane
  w momencie zakupu. Edycja lub ukrycie menu nie zmienia historii.
- **Miękkie ukrywanie menu** — `available = 0` zamiast `DELETE`, aby nie
  tracić referencji.
- **Brak kaskadowego usuwania użytkownika z zamówieniami** —
  `orders.user_id` ma `ON DELETE RESTRICT`; historia zamówień jest
  zachowana.
- **Tokeny tylko jako skróty** — sesje nie zawierają jawnego tokenu;
  `qr_token` jest jawny w bazie (potrzebny do generowania QR), ale baza
  powinna być chroniona jak każdy zbiór danych.

## 7. Frontend

- Czysty HTML/CSS/JS, bez frameworka, bez transpilacji, bez CDN.
- Wszystkie skrypty są zewnętrzne (`<script src>`), CSP nie dopuszcza
  kodu inline ani `eval`.
- `common.js`:
  - wrapper `fetch` z `credentials: same-origin` i `cache: no-store`,
  - automatyczne dodawanie `X-CSRF-Token`,
  - obsługa `401` (przekierowanie na login),
  - dialog wymuszonej zmiany hasła,
  - `bootPage(expectedRole, callback)` — pobiera `/api/auth/me`,
    sprawdza rolę, blokuje renderowanie przy `must_change_password`,
  - funkcje DOM (`h`, `textContent`, bez `innerHTML`), formatowanie PLN,
    dat, statusów, toasty.
- `staff-common.js` — wspólne komponenty kuchni i admina: menedżer menu,
  karty zamówień, wynik wyszukiwania pickup.
- `charts.js` — lekkie wykresy na canvasie (linia, słupki, słupki poziome),
  świadome DPR, responsywne, kolory z CSS variables; bez zewnętrznych
  bibliotek i bez CDN.
- `student.js` — menu, koszyk, checkout, potwierdzenie z QR, lista zamówień.
- `kitchen.js` — kolejka, formularz pickup, skaner QR kamery
  (`BarcodeDetector`, a gdy go brak — lokalny `vendor/jsQR.js` ładowany
  dynamicznie tylko na stronie kuchni) z fallbackiem na pole tekstowe.
- `vendor/jsQR.js` — jsQR 1.4.0 (Apache-2.0) serwowany z własnego
  serwera; bez CDN, bez inline JS, bez zmian w CSP. Szczegóły i sumy
  SHA-256: `frontend/vendor/README.md`.
- `admin.js` — statystyki, wykresy aktywności (7/14/30/90 dni), konta,
  generowanie, eksport CSV (fetch + blob), reset hasła, usuwanie konta z
  potwierdzeniem loginu, menu, zamówienia, ustawienia, audyt.
- Renderowanie opiera się na `textContent` i tworzeniu węzłów, co ogranicza
  ryzyko XSS; brak `innerHTML`, `eval`, `document.write`.

## 8. Baza danych i współbieżność

- SQLite w trybie **WAL** (`journal_mode=WAL`), `synchronous=NORMAL`,
  `busy_timeout=5s`, `foreign_keys=ON`.
- Pula połączeń `max_connections` (1–128, domyślnie 16); SQLite i tak
  serializuje zapisy, więc większa pula nie zwiększa przepustowości zapisu.
- Zapisy zamówienia w transakcji; odczyty równoległe.
- Indeksy pod kolejność zamówień, sesje, menu i audyt.
- Czasy w bazie: Unix UTC. `start_of_today()` liczy początek doby
  w `Europe/Warsaw` (statystyki dzienne), z poprawną obsługą DST.
- `pickup_time` to tekst `HH:MM` — godzina ścienna, bez strefy; wyświetlana
  bez konwersji.

## 9. Start aplikacji

```text
main()
  → logging::init() (JSON na stdout / journald)
  → Config::load_or_create("config.json" + .env)
      → walidacja DEPLOY_MODE, BIND_ADDR, TRUSTED_PROXIES, PUBLIC_URL
      → w tunnel: loopback + PUBLIC_URL wymagane, COOKIE_SECURE=true
  → db::init()
      → połączenie SQLite + PRAGMA
      → CREATE TABLE/INDEX IF NOT EXISTS
      → seed settings (INSERT OR IGNORE)
      → seed menu (tylko gdy tabela pusta)
      → cleanup (sesje wygasłe, audyt > 90 dni)
      → ensure_initial_admin (tylko gdy brak adminów)
  → dummy_hash (Argon2id)
  → load_or_create_jwt_secret("jwt.key") (0600, 64+ hex)
  → build_router(state)
  → TcpListener::bind(BIND_ADDR)
      └─ przy błędzie: czytelny błąd i zakończenie, bez fallbacku
  → axum::serve(...)
```

Aplikacja nie zmienia już w locie interfejsu nasłuchu. W trybie `tunnel`
odmówi startu, jeśli `BIND_ADDR` nie jest adresem pętlowym, a w obu
trybach zakończy się błędem, gdy port jest zajęty.

## 10. Decyzje i kompromisy

| Decyzja | Uzasadnienie | Kompromis |
| --- | --- | --- |
| SQLite zamiast PostgreSQL | zero administracji, łatwy backup, wystarcza dla jednej stołówki | jeden zapisujący proces, brak replikacji |
| własne sesje w DB | pełna kontrola, natychmiastowe unieważnianie | dodatkowy odczyt DB na żądanie |
| JWT tylko jako drugi kanał | wygoda klientów API bez rezygnacji z ciasteczek `HttpOnly` | dwa warianty w middleware, krótki TTL 15 min |
| sesja DB także dla JWT | natychmiastowa rewokacja (logout/zmiana hasła/dezaktywacja) | wpis w `sessions` na każdy token |
| SHA-256 tokenów sesji | wyciek bazy nie ujawnia ciasteczek | brak możliwości odzyskania tokenu (nie jest potrzebny) |
| CSRF + origin guard | ochrona podwójna dla przeglądarki | klienci Bearer są zwolnieni z CSRF |
| brak rejestracji | zero danych osobowych, kontrola kont | ręczne generowanie kont |
| brak e-mail/SMS | brak danych kontaktowych | brak samoobsługowego resetu |
| płatność przy ladzie | brak danych kart | brak przedpłaty online |
| snapshoty pozycji | historia odporna na zmiany menu | większe zużycie miejsca (nieistotne) |
| frontend bez build-stepu | prostota, mniejsza powierzchnia ataku, łatwy audyt | brak typowania i gotowych komponentów |
| jeden plik binarny | łatwe wdrożenie | brak hot-reload w produkcji |

## 11. Skalowanie

Rozwiązanie jest projektowane na jedną placówkę:

- dziesiątki–setki uczniów, kilka tysięcy zamówień dziennie w szczytach,
- SQLite WAL obsłuży wiele odczytów i serializowane zapisy,
- lista zamówień jest ograniczona (300), więc UI pozostaje szybkie,
- indeksy pokrywają typowe zapytania (status + czas, użytkownik + czas).

Przy większej skali należy rozważyć PostgreSQL, kolejkę/worker do
raportów i osobną warstwę cache, ale to poza zakresem obecnej wersji.
Nie należy uruchamiać wielu instancji aplikacji na tym samym pliku SQLite
przez NFS — SQLite nie jest do tego przeznaczony.

## 12. Testowalność

- Testy jednostkowe w modułach `config`, `security`, `auth`, `util`,
  `handlers::admin` (`cargo test`), w tym roundtrip JWT, odrzucenie
  złego sekretu/wygaśnięcia, escapowanie CSV i ochrona przed formułami.
- Logika handlerów jest cienka; większość walidacji jest jawna i łatwa do
  pokrycia testami integracyjnymi.
- Aplikacja nie wymaga frontendu do testów API — wystarczy `curl`.
- Dla testów E2E należy użyć osobnego katalogu roboczego (własny
  `config.json`, `baza.db`, port), aby nie dotknąć danych produkcyjnych.
