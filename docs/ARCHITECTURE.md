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
│  security_headers → origin_guard → body limit → routes       │
│                                                              │
│  publiczne:  /api/auth/login, /api/public/info               │
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
| `src/config.rs` | wczytywanie/tworzenie `config.json`, walidacja i normalizacja pól, `sqlite_url()` |
| `src/state.rs` | `AppState`: pula DB, konfiguracja, dummy hash, limiter logowania |
| `src/db.rs` | schemat, indeksy, seed ustawień i menu, czyszczenie, bootstrap admina, audyt, ustawienia |
| `src/auth.rs` | ciasteczka sesji, middleware sesji i CSRF, guardy ról, limiter logowania |
| `src/security.rs` | Argon2id, generatory loginów/haseł/kodów, SHA-256 tokenów, porównanie w czasie stałym |
| `src/logging.rs` | inicjalizacja `tracing` (stdout + dzienne pliki `logs/`), middleware logujące żądania bez danych wrażliwych |
| `src/util.rs` | czas UTC, początek dnia lokalnego, data/godzina lokalna, walidacja `HH:MM` |
| `src/error.rs` | `AppError`, spójne odpowiedzi JSON, mapowanie błędów SQL/Argon2/IO, logowanie błędów wewnętrznych |
| `src/handlers/auth.rs` | login, logout, `me`, zmiana hasła, logowanie zdarzeń bezpieczeństwa |
| `src/handlers/public.rs` | publiczne informacje o stołówce |
| `src/handlers/menu.rs` | menu ucznia i personelu, CRUD pozycji |
| `src/handlers/orders.rs` | składanie/anulowanie zamówień, QR, kolejka kuchni, pickup, zmiana statusów |
| `src/handlers/admin.rs` | statystyki, aktywność, konta, generowanie, reset haseł, usuwanie kont, eksport CSV, ustawienia, audyt |

## 4. Cykl żądania HTTP

Kolejność warstw (od zewnątrz):

0. **`request_log`** — mierzy czas i loguje metodę, ścieżkę bez query,
   status i czas odpowiedzi. Nie loguje nagłówków, ciasteczek, tokenów
   CSRF, ciał żądań ani adresów IP. Dla `/api/*` poziom zależy od statusu
   (info / warn / error), dla statyków sukces jest na poziomie `debug`.
1. **`security_headers`** — dodaje CSP, `X-Content-Type-Options`,
   `X-Frame-Options`, `Referrer-Policy`, `Permissions-Policy`,
   COOP/CORP, opcjonalnie HSTS; dla `/api/*` dodaje
   `Cache-Control: no-store`.
2. **`origin_guard`** — dla metod innych niż `GET`/`HEAD`/`OPTIONS`:
   blokuje `Sec-Fetch-Site: cross-site` oraz niezgodny `Origin` vs `Host`.
3. **`RequestBodyLimitLayer`** — limit 64 KiB.
4. **Router** — dopasowanie ścieżki i metody; statyczne pliki z
   `frontend/` przez `ServeDir`, `/` przekierowuje na `/login`.
5. **`auth_middleware`** (tylko trasy chronione) —
   - odczyt ciasteczka `zsl_session`,
   - walidacja formatu (64 hex),
   - SHA-256 tokenu i odczyt sesji + użytkownika z bazy,
   - sprawdzenie wygaśnięcia i `active`,
   - weryfikacja CSRF dla metod zmieniających stan,
   - blokada kont z `must_change_password`,
   - odświeżenie `last_seen_at` maks. raz na 5 minut,
   - wstawienie `AuthUser` do rozszerzeń żądania.
6. **Guard roli** (`require_student`, `require_staff`, `require_admin`) —
   porównanie `AuthUser.role`.
7. **Handler** — walidacja, logika, SQL, audyt, odpowiedź JSON.

Dzięki `auth_middleware` handlery nie muszą same sprawdzać sesji ani CSRF.
Guardy ról są nakładane przez `route_layer` na poddrzewa tras, więc nowy
endpoint w `/api/admin` automatycznie dziedziczy wymóg roli `admin`.

## 5. Przepływy

### 5.1. Logowanie

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

### 5.2. Złożenie zamówienia

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

### 5.3. QR i odbiór

- QR generuje serwer w `GET /api/student/orders/:id/qr.svg` na podstawie
  `qr_token`; w obrazie jest `ZSL:<token>`, bez danych ucznia.
- Kuchnia skanuje QR (`BarcodeDetector` + `getUserMedia`) albo wpisuje
  kod. `POST /api/staff/pickup` normalizuje krótki kod do wielkich liter,
  opcjonalnie usuwa prefiks `ZSL:` i szuka po `qr_token` lub `code`.
- Pickup **nie zmienia** statusu — służy do identyfikacji zamówienia.
  Wydanie następuje przez `PATCH /api/staff/orders/:id`.
- Każde wyszukanie trafia do audytu.

### 5.4. Zmiana statusu i płatności

- Kuchnia: `new → preparing → ready → completed` (lub pominięcie stanów),
  `completed` automatycznie ustawia `paid`.
- Kuchnia nie może modyfikować zamówień `completed`/`cancelled`.
- Student może anulować tylko własne `new`.
- Administrator może ustawić dowolny dozwolony status i płatność.

### 5.5. Statystyki aktywności

`GET /api/admin/stats/activity?days=1..90`:

```text
→ wybierz zakres [początek dnia lokalnego (dziś - days + 1), teraz]
→ jedno zapytanie o zamówienia z zakresu (created_at, status, payment_status, total)
→ agregacja w Rust:
    ├─ daily[] — liczba, ukończone, anulowane, utarg (tylko paid i nie-cancelled)
    ├─ hourly_today[] — 24 kubełki dla dzisiejszych zamówień
    ├─ status_counts / payment_counts
    └─ totals (orders, completed, cancelled, revenue, średnia)
→ drugie zapytanie: TOP 8 pozycji (JOIN order_items, bez anulowanych)
→ JSON gotowy do wykresów canvas
```

Dni są grupowane według **strefy lokalnej serwera**, a znaczniki czasu w
bazie pozostają w UTC. Statystyki nie zawierają danych osobowych.

### 5.6. Twarde usuwanie konta

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

### 5.7. Eksport CSV metadanych

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

### 5.8. Logowanie techniczne

- `logging::init()` tworzy `logs/` i dzienne pliki
  `logs/zsl_order.log.RRRR-MM-DD` (`tracing-appender`).
- Domyślny filtr: `info,sqlx=warn,hyper=warn,axum=warn`; można nadpisać
  przez `RUST_LOG`.
- Logi trafiają równolegle na stdout i do pliku.
- Zdarzenia: start/konfiguracja/baza/serwer, logowania (sukces, brak konta,
  błędne hasło, limiter), zmiany haseł, generowanie/aktualizacja/usuwanie
  kont, reset hasła, eksport CSV, błędy wewnętrzne i błędy zapisu audytu.
- **Nigdy** nie logujemy haseł, tokenów sesji/CSRF, ciasteczek, ciał żądań
  ani adresów IP. Logowanie żądań nie zapisuje też parametrów zapytania.

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
- `kitchen.js` — kolejka, formularz pickup, skaner QR kamery (jeśli
  `BarcodeDetector` jest dostępny) z fallbackiem na pole tekstowe.
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
- Czasy w bazie: Unix UTC. `start_of_today()` liczy początek dnia
  w strefie lokalnej serwera (statystyki dzienne).
- `pickup_time` to tekst `HH:MM` — godzina ścienna, bez strefy; wyświetlana
  bez konwersji.

## 9. Start aplikacji

```text
main()
  → logging::init() (stdout + logs/)
  → Config::load_or_create("config.json")
  → db::init()
      → połączenie SQLite + PRAGMA
      → CREATE TABLE/INDEX IF NOT EXISTS
      → seed settings (INSERT OR IGNORE)
      → seed menu (tylko gdy tabela pusta)
      → cleanup (sesje wygasłe, audyt > 90 dni)
      → ensure_initial_admin (tylko gdy brak adminów)
  → dummy_hash (Argon2id)
  → build_router(state)
  → TcpListener::bind(ip:port)
      └─ przy błędzie fallback na 0.0.0.0:3000 (z komunikatem)
  → axum::serve(...)
```

Fallback na `0.0.0.0:3000` istnieje dla wygody lokalnej, ale w produkcji
lepiej dopilnować, aby wybrany port był wolny; wtedy aplikacja nie zmieni
nieoczekiwanie interfejsu nasłuchu. Zobacz też ograniczenia w
[SECURITY.md](SECURITY.md).

## 10. Decyzje i kompromisy

| Decyzja | Uzasadnienie | Kompromis |
| --- | --- | --- |
| SQLite zamiast PostgreSQL | zero administracji, łatwy backup, wystarcza dla jednej stołówki | jeden zapisujący proces, brak replikacji |
| własne sesje w DB | pełna kontrola, natychmiastowe unieważnianie | dodatkowy odczyt DB na żądanie |
| SHA-256 tokenów sesji | wyciek bazy nie ujawnia ciasteczek | brak możliwości odzyskania tokenu (nie jest potrzebny) |
| CSRF + origin guard | ochrona podwójna, także dla klientów bez CSRF | wymaga poprawnego nagłówka w klientach API |
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

- Testy jednostkowe w modułach `config`, `security`, `util`, `handlers::admin`
  (`cargo test`), w tym escapowanie CSV i ochrona przed formułami.
- Logika handlerów jest cienka; większość walidacji jest jawna i łatwa do
  pokrycia testami integracyjnymi.
- Aplikacja nie wymaga frontendu do testów API — wystarczy `curl`.
- Dla testów E2E należy użyć osobnego katalogu roboczego (własny
  `config.json`, `baza.db`, port), aby nie dotknąć danych produkcyjnych.
