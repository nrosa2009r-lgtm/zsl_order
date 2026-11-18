# Baza danych — ZSL Order

Aplikacja używa SQLite w trybie WAL. Schemat jest zarządzany przez
wbudowane migracje `sqlx` (`migrations/*.sql`, wykonywane przy starcie).
Migracja bazowa używa `CREATE TABLE/INDEX IF NOT EXISTS`, więc działa
również na bazie utworzonej przez starszą wersję aplikacji i nie usuwa
danych. Wersje są zapisywane w `_sqlx_migrations`. Nie ma zewnętrznego
serwera bazy.

- Plik domyślnie: `baza.db` w katalogu roboczym.
- Połączenie: `sqlite://baza.db` (konfiguracja `db_url`).
- `foreign_keys = ON`, `journal_mode = WAL`, `synchronous = NORMAL`,
  `busy_timeout = 5 s`.
- Czasy: `INTEGER` — Unix UTC w sekundach.
- Wartości logiczne: `INTEGER` 0/1.
- Kwoty: `INTEGER` w groszach.

## 1. Diagram relacji

```text
users (1) ───< (N) sessions
users (1) ───< (N) orders (1) ───< (N) order_items
menu_items (1) ───< (N) order_items        (opcjonalnie, SET NULL)
users (1) ───< (N) audit_log               (actor_id, opcjonalnie)
settings — tabela key/value
```

- `sessions.user_id` → `users.id` `ON DELETE CASCADE`
- `orders.user_id` → `users.id` `ON DELETE SET NULL`
- `order_items.order_id` → `orders.id` `ON DELETE CASCADE`
- `order_items.menu_item_id` → `menu_items.id` `ON DELETE SET NULL`
- `audit_log.actor_id` → brak twardego FK (celowo, aby audyt przetrwał
  usunięcie konta; w praktyce konta nie są usuwane)

## 2. Tabele

### `users`

```sql
CREATE TABLE IF NOT EXISTS users (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    login TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    role TEXT NOT NULL CHECK (role IN ('student','kitchen','admin')),
    display_code TEXT NOT NULL UNIQUE,
    active INTEGER NOT NULL DEFAULT 1,
    must_change_password INTEGER NOT NULL DEFAULT 0,
    class_group TEXT,
    created_at INTEGER NOT NULL,
    last_login_at INTEGER,
    uncollected_block_reset_at INTEGER
);
CREATE INDEX IF NOT EXISTS users_role_idx ON users(role);
CREATE INDEX IF NOT EXISTS users_class_group_idx ON users(class_group);
```

| Kolumna | Opis |
| --- | --- |
| `id` | techniczny identyfikator |
| `login` | losowy login, unikalny, np. `s-a7k2m9p4qx` |
| `password_hash` | Argon2id (PHC string z solą) |
| `role` | `student`, `kitchen`, `admin` |
| `display_code` | pseudonimowy kod publiczny, np. `UCZ-AB12C`, unikalny |
| `active` | 0/1; 0 blokuje logowanie i unieważnia sesje przy zmianie |
| `must_change_password` | 0/1; 1 blokuje API poza zmianą hasła |
| `class_group` | pseudonim klasy z importu CSV (np. `1A`) lub `NULL` |
| `created_at` | Unix UTC |
| `last_login_at` | Unix UTC, może być `NULL` |
| `uncollected_block_reset_at` | Unix UTC; od tej chwili liczy się blokada nieodebranych po akcji „zdejmij blokadę” |

Brak pól osobowych: imienia, nazwiska, e-maila, telefonu itp.
`class_group` to wyłącznie krótki pseudonim klasy/grupy z importu
(`1A`, `2B`), bez numeru ucznia i bez powiązania z konkretną osobą.

### `sessions`

```sql
CREATE TABLE IF NOT EXISTS sessions (
    token_hash TEXT PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    csrf_token TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    last_seen_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS sessions_user_idx ON sessions(user_id);
CREATE INDEX IF NOT EXISTS sessions_expiry_idx ON sessions(expires_at);
```

| Kolumna | Opis |
| --- | --- |
| `token_hash` | SHA-256 tokenu sesji (hex) dla cookie albo SHA-256(`"jwt:" + jti`) dla tokenu JWT; jawny token tylko u klienta |
| `user_id` | właściciel sesji |
| `csrf_token` | losowy token CSRF (jawny, potrzebny do porównania); pusty dla wierszy JWT |
| `created_at` | Unix UTC |
| `expires_at` | Unix UTC; dla JWT `now + 900` |
| `last_seen_at` | Unix UTC; odświeżane maks. co 5 min |

Tabela obsługuje **dwa rodzaje sesji**: ciasteczkowe (TTL
`session_hours`) i JWT (TTL 15 min). Wiersz JWT jest tworzony przy
`POST /api/auth/token`, a usuwany przy logout Bearer, zmianie hasła,
dezaktywacji konta lub twardym usunięciu użytkownika. Dzięki temu token
JWT działa jak referencja do sesji i można go unieważnić natychmiast.

Wygasłe sesje (oba rodzaje) są usuwane przy logowaniu, wydaniu tokenu
i starcie aplikacji.

### `menu_items`

```sql
CREATE TABLE IF NOT EXISTS menu_items (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    price_cents INTEGER NOT NULL CHECK (price_cents >= 0),
    category TEXT NOT NULL DEFAULT 'Dania',
    allergen_codes TEXT NOT NULL DEFAULT '',
    available INTEGER NOT NULL DEFAULT 1,
    sort_order INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    stock_remaining INTEGER
        CHECK (stock_remaining IS NULL OR stock_remaining >= 0),
    stock_default INTEGER
        CHECK (stock_default IS NULL OR stock_default >= 0),
    stock_date TEXT
);
CREATE INDEX IF NOT EXISTS menu_available_idx ON menu_items(available, sort_order);
CREATE INDEX IF NOT EXISTS menu_items_stock_date_idx ON menu_items(stock_date);
```

- `available = 0` to miękkie ukrycie; pozycja pozostaje w historii.
- `allergen_codes` — lista kodów 14 alergenów UE po przecinkach, zawsze
  w kolejności katalogowej i bez duplikatów (`gluten,celery`); pusty
  string = brak alergenów. W API pole jest tablicą. Stare pole tekstowe
  `allergens` zostało usunięte przez migrację 0009 (patrz sekcja 8).
- `sort_order` steruje kolejnością; mniejsza wartość = wyżej.
- Pozycje są snapshotowane do `order_items` w momencie zamówienia.
- `stock_remaining` to stan na dzień `stock_date` (`RRRR-MM-DD`,
  Europe/Warsaw); `NULL` = bez limitu, `0` = wyprzedane.
- `stock_default` to stan, do którego pozycja wraca przy pierwszym
  odczycie nowego dnia. `NULL` = brak automatycznego resetu (stan
  ustawiony ręcznie obowiązuje do następnej zmiany). Samo ustawienie
  `stock_default` również zapisuje `stock_date = dziś`, więc limit
  zacznie obowiązywać od następnego dnia, nawet jeśli dziś pozycja jest
  bez limitu.
- Reset jest **leniwy**: wykonywany w transakcji przy pierwszym odczycie
  menu, zamówieniu lub raporcie nowego dnia. Nie ma zadania w tle.
- `CHECK` blokuje wartości ujemne także w razie błędu w kodzie.

### `orders`

```sql
CREATE TABLE IF NOT EXISTS orders (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id INTEGER REFERENCES users(id) ON DELETE SET NULL,
    code TEXT NOT NULL UNIQUE,
    qr_token TEXT NOT NULL UNIQUE,
    status TEXT NOT NULL DEFAULT 'ordered'
        CHECK (status IN ('ordered','issued','cancelled','uncollected')),
    payment_method TEXT NOT NULL CHECK (payment_method IN ('cash','card')),
    payment_status TEXT NOT NULL DEFAULT 'unpaid'
        CHECK (payment_status IN ('unpaid','paid')),
    total_cents INTEGER NOT NULL CHECK (total_cents >= 0),
    pickup_time TEXT NOT NULL DEFAULT '',
    pickup_date TEXT,
    break_id INTEGER REFERENCES breaks(id) ON DELETE SET NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    paid_at INTEGER,
    paid_by INTEGER,
    ready_at INTEGER,
    issued_at INTEGER,
    uncollected_at INTEGER,
    idempotency_key TEXT
);
CREATE INDEX IF NOT EXISTS orders_user_idx ON orders(user_id, created_at);
CREATE INDEX IF NOT EXISTS orders_status_idx ON orders(status, created_at);
CREATE INDEX IF NOT EXISTS orders_uncollected_idx ON orders(uncollected_at);
CREATE INDEX IF NOT EXISTS orders_ready_idx ON orders(status, ready_at);
CREATE INDEX IF NOT EXISTS orders_break_idx ON orders(break_id, created_at);
CREATE INDEX IF NOT EXISTS orders_pickup_idx ON orders(pickup_date, break_id);
CREATE UNIQUE INDEX IF NOT EXISTS orders_user_idempotency_idx
    ON orders(user_id, idempotency_key)
    WHERE idempotency_key IS NOT NULL;
```

| Kolumna | Opis |
| --- | --- |
| `user_id` | właściciel (pseudonimowe konto); `NULL` po usunięciu konta |
| `code` | 6 wielkich liter, unikalny kod odbioru |
| `qr_token` | 48 znaków hex (24 B), unikalny token QR |
| `status` | `ordered`, `issued`, `cancelled`, `uncollected` |
| `payment_method` | `cash`, `card` |
| `payment_status` | `unpaid`, `paid` |
| `total_cents` | suma policzona po stronie serwera |
| `pickup_time` | `""` lub `HH:MM` (godzina lokalna) |
| `pickup_date` | dzień odbioru `RRRR-MM-DD` (strefa szkoły); `NULL` tylko dla zamówień sprzed migracji 0008 — wtedy obowiązuje dzień złożenia |
| `break_id` | przerwa, na którą złożono zamówienie; `NULL` gdy brak lub usunięta |
| `created_at`, `updated_at` | Unix UTC |
| `paid_at` | czas oznaczenia płatności (Unix UTC) lub `NULL` |
| `paid_by` | techniczne ID konta, które oznaczyło płatność (bez FK) |
| `ready_at` | znacznik historyczny z migracji starych statusów; nowe zamówienia go nie ustawiają |
| `issued_at` | moment wydania zamówienia |
| `uncollected_at` | znacznik nieodebrania (status `uncollected`) |
| `idempotency_key` | klucz klienta, unikalny per `user_id` (indeks częściowy) |

`uncollected` jest osobnym statusem w bazie. Zamówienie nieodebrane nie
jest liczone jednocześnie jako anulowane ani opłacone w kategoriach
liczbowych raportu (opłacone nieodebrane zostaje natomiast w utargu).

`user_id` dopuszcza `NULL`, a klucz obcy ma `ON DELETE SET NULL`: usunięcie
konta nie kasuje historii zamówień. API pokazuje wtedy `student_code =
"konto usunięte"`, a `student` jest puste. `break_id` analogicznie zeruje
się przy usunięciu przerwy.

### `breaks`

```sql
CREATE TABLE IF NOT EXISTS breaks (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL CHECK (length(trim(name)) BETWEEN 1 AND 60),
    start_time TEXT NOT NULL CHECK (start_time GLOB '[0-2][0-9]:[0-5][0-9]'),
    end_time TEXT NOT NULL CHECK (end_time GLOB '[0-2][0-9]:[0-5][0-9]'),
    active INTEGER NOT NULL DEFAULT 1 CHECK (active IN (0, 1)),
    sort_order INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    CHECK (start_time < end_time)
);
CREATE UNIQUE INDEX IF NOT EXISTS breaks_name_idx ON breaks(name);
CREATE INDEX IF NOT EXISTS breaks_active_idx ON breaks(active, sort_order);
```

Przerwy definiują okna wydawania zamówień (np. „Długa przerwa”
11:30–11:50). `name` jest unikalna, godziny w formacie `HH:MM` czasu
szkoły, a `CHECK` wymaga `start_time < end_time`. Tabela powstaje
w migracji 0007, a API i egzekwowanie limitów „na przerwę” w 0008.
Przerwy nie są seedowane — administrator dodaje je ręcznie przed
pilotażem; bez aktywnej przerwy uczeń nie wybiera okna, a limity
`*_per_break` nie działają. Usunięcie przerwy z historią jest blokowane
przez API (`409`); zamiast tego przerwę należy wyłączyć (`active = 0`).

Uwaga: `qr_token` jest przechowywany jawnie, ponieważ serwer musi
wygenerować obraz QR. Ochrona pliku bazy jest ochroną kodów odbioru.

### `order_items`

```sql
CREATE TABLE IF NOT EXISTS order_items (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    order_id INTEGER NOT NULL REFERENCES orders(id) ON DELETE CASCADE,
    menu_item_id INTEGER REFERENCES menu_items(id) ON DELETE SET NULL,
    name TEXT NOT NULL,
    price_cents INTEGER NOT NULL CHECK (price_cents >= 0),
    quantity INTEGER NOT NULL CHECK (quantity > 0),
    stock_reserved INTEGER NOT NULL DEFAULT 0
        CHECK (stock_reserved IN (0, 1))
);
CREATE INDEX IF NOT EXISTS order_items_order_idx ON order_items(order_id);
```

- `name` i `price_cents` to **snapshot** z momentu zakupu.
- `menu_item_id` może stać się `NULL`, jeśli pozycja menu zostanie
  kiedykolwiek fizycznie usunięta (aplikacja tylko ukrywa).
- `quantity` 1–10 (walidacja aplikacji; baza wymaga > 0).
- `stock_reserved = 1` oznacza, że pozycja faktycznie zabrała limitowany
  stan. Anulowanie zwraca sztuki tylko takim pozycjom — limit ustawiony
  po złożeniu zamówienia nie zawyża stanu.

### `settings`

```sql
CREATE TABLE IF NOT EXISTS settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
```

Domyślne wpisy (seed `INSERT OR IGNORE`):

| Klucz | Domyślnie | Znaczenie |
| --- | --- | --- |
| `cafeteria_name` | `Stołówka ZSŁ` | nazwa wyświetlana |
| `ordering_enabled` | `1` | `1`/`0` — składanie zamówień |
| `payment_info` | `Płatność przy odbiorze: gotówka lub karta.` | informacja dla ucznia |
| `max_open_orders` | `3` | limit otwartych zamówień na użytkownika |
| `max_order_items` | `20` | limit pozycji w zamówieniu |
| `cancel_window_minutes` | `30` | termin anulowania od złożenia; `0` = bez limitu |
| `pickup_limit_minutes` | `120` | koniec przerwy odbioru + tyle minut = termin nieodebrania; `0` = wyłączone |
| `max_uncollected` | `0` | blokada po N nieodebranych z 30 dni; `0` = brak |
| `order_days_ahead` | `0` | ile dni naprzód można zamawiać; `0` = tylko dziś |
| `max_item_quantity_per_break` | `0` | limit sztuk jednej pozycji na przerwę; `0` = bez limitu |
| `one_active_order_per_break` | `0` | `1` = jedno aktywne zamówienie na przerwę |

Klucze `*_per_break` są egzekwowane w transakcji `create_order` dla pary
(uczeń, `pickup_date`, `break_id`) i dotyczą zamówień w statusie
`ordered`/`issued`. Gdy `order_days_ahead > 0`, zamówienia przyszłe nie
rezerwują dzisiejszego stanu magazynowego — stan jest kontrolowany
wyłącznie dla zamówień na dziś.

Wartości są stringami; API administracyjne waliduje zakresy i konwertuje
`ordering_enabled` z bool na `1`/`0`.

### `audit_log`

```sql
CREATE TABLE IF NOT EXISTS audit_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    actor_id INTEGER,
    action TEXT NOT NULL,
    details TEXT NOT NULL DEFAULT '',
    created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS audit_created_idx ON audit_log(created_at);
```

- `actor_id` — techniczny ID konta lub `NULL` (zdarzenie systemowe).
- `action` — np. `login`, `order_created`, `pickup_lookup`.
- `details` — techniczny opis (ID zamówienia, statusy, kwota); bez danych
  osobowych.
- Retencja: wpisy starsze niż 90 dni usuwane przy starcie.

## 3. Indeksy

| Indeks | Tabela | Kolumny | Zastosowanie |
| --- | --- | --- | --- |
| `users_role_idx` | `users` | `role` | filtry kont, statystyki |
| `sessions_user_idx` | `sessions` | `user_id` | wylogowanie wszystkich sesji |
| `sessions_expiry_idx` | `sessions` | `expires_at` | czyszczenie wygasłych |
| `menu_available_idx` | `menu_items` | `available, sort_order` | menu ucznia |
| `menu_items_stock_date_idx` | `menu_items` | `stock_date` | leniwy reset stanu |
| `orders_user_idx` | `orders` | `user_id, created_at` | historia ucznia |
| `orders_status_idx` | `orders` | `status, created_at` | kolejka kuchni |
| `orders_ready_idx` | `orders` | `status, ready_at` | historyczne znaczniki gotowości |
| `orders_break_idx` | `orders` | `break_id, created_at` | zamówienia przerwy |
| `orders_pickup_idx` | `orders` | `pickup_date, break_id` | raport dnia, limity przerwy |
| `orders_uncollected_idx` | `orders` | `uncollected_at` | licznik nieodebranych |
| `orders_user_idempotency_idx` | `orders` | `user_id, idempotency_key` (częściowy) | ochrona przed duplikatem |
| `order_items_order_idx` | `order_items` | `order_id` | składanie zamówień |
| `breaks_name_idx` | `breaks` | `name` (unikalny) | nazwy przerw |
| `breaks_active_idx` | `breaks` | `active, sort_order` | aktywne przerwy |
| `audit_created_idx` | `audit_log` | `created_at` | retencja, lista |

Unikalne ograniczenia (`UNIQUE`) tworzą dodatkowe indeksy na `users.login`,
`users.display_code`, `orders.code`, `orders.qr_token`.

## 4. Seed i bootstrap

Przy każdym starcie:

1. Migracje `sqlx` (wersje z `migrations/`, m.in. tabele i indeksy).
2. `settings`: wstawienie brakujących domyślnych kluczy.
3. Menu: seed 7 pozycji **tylko gdy tabela jest pusta**.
4. Czyszczenie: sesje wygasłe, audyt > 90 dni.
5. Startowy admin: tylko gdy `COUNT(*) FROM users WHERE role='admin'` = 0.

Startowy admin:

- login `admin-<10 znaków>`,
- hasło 18 znaków,
- `display_code` `ADM-XXXXX`,
- `must_change_password = 1`,
- dane w konsoli i `admin_start.txt` (`0600`),
- zdarzenie `admin_seeded` w audycie.

## 5. Typowe zapytania operacyjne

Statystyki dnia:

```sql
SELECT COUNT(*) FROM orders WHERE created_at >= :start;
SELECT COALESCE(SUM(total_cents), 0) FROM orders
 WHERE created_at >= :start AND payment_status = 'paid' AND status <> 'cancelled';
```

Kolejka kuchni:

```sql
SELECT id, code, status, payment_method, total_cents, pickup_time
FROM orders
WHERE status = 'ordered'
ORDER BY created_at ASC, id ASC;
```

Historia ucznia:

```sql
SELECT id, code, status, total_cents, created_at
FROM orders WHERE user_id = ? ORDER BY created_at DESC, id DESC;
```

Wyszukanie pickup:

```sql
SELECT id FROM orders WHERE qr_token = ? OR code = ? LIMIT 1;
```

Wygasłe sesje:

```sql
DELETE FROM sessions WHERE expires_at < strftime('%s','now');
```

Unieważnienie wszystkich sesji (np. incydent):

```sql
DELETE FROM sessions;
```

Powyższe usuwa także wiersze JWT — wszystkie wydane tokeny dostępu
natychmiast przestają działać (oprócz tych, które i tak wygasły).
Unieważnienie pojedynczego tokenu wymaga znajomości `jti`; praktycznie
wygodniej wylogować sesję przez API lub usunąć wiersze danego
użytkownika:

```sql
DELETE FROM sessions WHERE user_id = ?;
```

Twarde usunięcie konta (to samo, co robi `DELETE /api/admin/users/:id`
oraz `POST /api/admin/users/bulk-delete`) wykonuje się w jednej
transakcji `BEGIN IMMEDIATE`:

```sql
BEGIN;
UPDATE orders SET user_id = NULL WHERE user_id = ?;  -- historia zostaje
DELETE FROM sessions WHERE user_id = ?;              -- plus kaskada FK
DELETE FROM users   WHERE id = ?;
COMMIT;
```

`orders.user_id` ma `ON DELETE SET NULL`, więc usunięcie konta zachowuje
historię zamówień — API pokazuje je jako „konto usunięte"
(`COALESCE(u.display_code, 'konto usunięte')`). Konto usunięte traci
login, hash hasła i sesje; raporty i audyt pozostają kompletne
(`audit_log` bez FK na `actor_id`, w widoku z `LEFT JOIN` na `users`).
Zabronione jest usunięcie własnego konta i ostatniego aktywnego
administratora; przy akcjach masowych dodatkowo chronione są wszystkie
aktywnych administratorzy (własne konto i aktywni admini nie wchodzą
do wyboru, a lista ID je zawierająca → `409`). Zdarzenie trafia do
audytu jako `user_deleted` (pojedyncze) lub zbiorczo
`bulk_users_deleted` / `bulk_users_deactivated`.

Dezaktywacja (`active = 0`) nie rusza historii ani danych konta — usuwa
tylko sesje i blokuje logowanie (tak samo w trybie masowym).

## 6. Retencja i porządki

Automatyczne:

- sesje wygasłe (cookie i JWT) — przy logowaniu, wydaniu tokenu i starcie,
- audyt starszy niż 90 dni — przy starcie.

Ręczne (zalecane, zgodnie z polityką szkoły):

- zamówienia starsze niż rok szkolny: usuń lub zanonimizuj,
- konta nieaktywne po zakończeniu roku: dezaktywuj, a po okresie retencji
  usuń (historia zamówień może zostać dzięki `orders.user_id ON DELETE
  SET NULL`; zdecyduj, czy ją zanonimizować).

Przykład anonymizacji zamówień starszych niż 365 dni bez usuwania
agregatów:

```sql
-- Najpierw backup!
UPDATE orders
SET user_id = (SELECT id FROM users WHERE login = 'deleted-account')
WHERE created_at < strftime('%s','now') - 365*24*3600;
```

W praktyce trzeba utworzyć konto techniczne `deleted-account` i wcześniej
zdecydować, czy historia ma pozostać. Alternatywnie usuń pozycje i
zamówienia w transakcji:

```sql
BEGIN;
DELETE FROM order_items WHERE order_id IN (
  SELECT id FROM orders WHERE created_at < strftime('%s','now') - 365*24*3600
);
DELETE FROM orders WHERE created_at < strftime('%s','now') - 365*24*3600;
COMMIT;
```

Usunięcie konta nie kasuje historii: `orders.user_id` ma
`ON DELETE SET NULL`, a API pokazuje `konto usunięte`. Konto można więc
usunąć bez utraty raportów; szczegóły akcji administracyjnych opisuje
`API.md`.

## 7. Backup i integralność

Procedura przed każdą aktualizacją, która zawiera migrację (0006, 0007,
0008, 0009, kolejne):

```bash
# 1. Zatrzymaj aplikację, żeby nikt nie pisał do bazy.
sudo systemctl stop zsl-order        # instalacja natywna
# docker compose stop zsl-order      # Docker

# 2. Wykonaj spójną kopię (nie zwykłym cp — WAL może być niespójny).
sqlite3 /var/lib/zsl-order/baza.db \
  ".backup '/var/backups/zsl-order/baza-przed-migracja.db'"

# 3. Sprawdź kopię, zanim uruchomisz nową wersję.
sqlite3 /var/backups/zsl-order/baza-przed-migracja.db "PRAGMA integrity_check;"
sqlite3 /var/backups/zsl-order/baza-przed-migracja.db "PRAGMA foreign_key_check;"

# 4. Uruchom nową wersję i sprawdź logi migracji.
sudo systemctl start zsl-order
journalctl -u zsl-order -n 50 --no-pager   # docker compose logs zsl-order
```

Zasady:

- Backup: `sqlite3 baza.db ".backup 'kopia.db'"` lub `VACUUM INTO`.
- Nie kopiuj `baza.db` przez `cp` podczas pracy — WAL może być niespójny.
- Po odtworzeniu usuń `baza.db-wal` i `baza.db-shm`.
- Sprawdzenie integralności po migracji i po odtworzeniu:

```bash
sqlite3 baza.db "PRAGMA integrity_check;"
sqlite3 baza.db "PRAGMA foreign_key_check;"
```

- Backup zawiera hash haseł, tokeny sesji i tokeny QR — traktuj jak dane
  wrażliwe.

## 8. Migracje

Migracje są plikami `migrations/RRRRMMDDHHMM_opis.sql`, wbudowanymi w
binarkę przez `sqlx::migrate!` i wykonywanymi przy starcie w kolejności
wersji. Zastosowane wersje i sumy kontrolne trafiają do
`_sqlx_migrations`; zmiana już zastosowanego pliku jest błędem.

Zasady:

1. **Zawsze backup przed migracją** — `sqlite3 baza.db ".backup ..."`.
2. Testuj migrację na kopii bazy z danymi oraz na pustej bazie.
3. Preferuj zmiany addytywne (`ALTER TABLE ... ADD COLUMN`,
   `CREATE INDEX IF NOT EXISTS`).
4. SQLite ma ograniczone `ALTER TABLE`; przebudowa tabeli wymaga
   utworzenia nowej, kopiowania danych i podmiany w transakcji.
5. Aplikacja przy starcie nie usuwa ani nie modyfikuje istniejących
   danych; seed menu działa tylko na pustej tabeli.
6. Migracja bazowa (`0001`) jest idempotentna i obsługuje bazy starsze
   niż `_sqlx_migrations`.

`0002_users_class_group.sql` dodaje `users.class_group` i uzupełnia je
z loginów zaimportowanych kont (`s-1a-...` → `1A`). Zwykłe konta
`s-...` pozostają z `NULL`.

`0003_orders_payments.sql` dodaje pola płatności/odbioru oraz
`idempotency_key` z częściowym indeksem unikalnym. Dla istniejących
opłaconych zamówień ustawia `paid_at = updated_at` (historyczny operator
pozostaje nieznany, więc `paid_by = NULL`).

`0004_uncollected_block_reset.sql` dodaje
`users.uncollected_block_reset_at`. Dzięki temu administrator może zdjąć
blokadę nieodebranych bez kasowania historii zamówień; licznik blokady
uwzględnia tylko zamówienia nieodebrane po tej dacie.

`0005_retention_indexes.sql` dodaje indeksy `orders_created_idx` oraz
częściowy `orders_idempotency_created_idx`, wspierające nocną retencję.

`0006_menu_stock.sql` dodaje dzienny stan magazynowy
(`menu_items.stock_remaining`, `stock_default`, `stock_date`),
`order_items.stock_reserved` (0/1) i indeks `menu_items_stock_date_idx`.
Wszystkie zmiany są addytywne: istniejące pozycje dostają `NULL`
(bez limitu), a historyczne pozycje zamówień `stock_reserved = 0`.
Przed migracją wykonaj backup i sprawdź po niej `PRAGMA
foreign_key_check;` oraz `PRAGMA integrity_check;`.

`0007_orders_rebuild_breaks.sql` przebudowuje `orders` (SQLite nie
pozwala zmienić `CHECK` ani kluczy obcych przez `ALTER TABLE`):

- nowe statusy `ordered`, `issued`, `cancelled`, `uncollected`,
- `issued_at` — moment wydania; dla historycznych `completed` brany jest
  `paid_at`, potem `ready_at`, a na końcu `updated_at`,
- `user_id` może być `NULL` (`ON DELETE SET NULL`), więc usunięcie konta
  nie kasuje historii zamówień,
- `break_id` → `breaks(id)` (`ON DELETE SET NULL`) i tabela `breaks`.

Mapowanie starych statusów:

| Stary | Nowy |
| --- | --- |
| `new`, `preparing`, `ready` | `ordered` |
| `completed` | `issued` |
| `cancelled` z `uncollected_at` | `uncollected` |
| `cancelled` bez `uncollected_at` | `cancelled` |

`ready_at` zostaje jako znacznik historyczny; nowe zamówienia korzystają
z `issued_at`. Migracja jest odporna na ponowne uruchomienie: `CASE`
rozpoznaje także wartości nowego schematu, a tabele powstają przez
`CREATE TABLE IF NOT EXISTS`.

Procedura migracji 0007 (wykonaj **przed** startem nowej wersji):

```bash
# 1. Zatrzymaj aplikację i zrób spójną kopię.
sqlite3 baza.db ".backup 'baza-przed-0007.db'"

# 2. Zapisz liczby kontrolne.
sqlite3 baza.db "SELECT COUNT(*) FROM orders;"
sqlite3 baza.db "SELECT COUNT(*) FROM order_items;"
sqlite3 baza.db "SELECT COALESCE(SUM(total_cents),0) FROM orders;"

# 3. Uruchom nową wersję (migracja przy starcie), potem sprawdź:
sqlite3 baza.db "PRAGMA foreign_key_check;"
sqlite3 baza.db "PRAGMA integrity_check;"
sqlite3 baza.db "SELECT status, COUNT(*) FROM orders GROUP BY status;"
sqlite3 baza.db "SELECT COUNT(*) FROM order_items;"
sqlite3 baza.db "SELECT COALESCE(SUM(total_cents),0) FROM orders;"

# 4. Porównaj liczby z kroku 2. W razie różnicy odtwórz kopię.
```

Migracja nie tworzy widoków ani triggerów; jedyne klucze obce do `orders`
to `order_items.order_id` (kaskadowy) i `orders.user_id`/`break_id`.
Przebudowa kopiuje `order_items` razem z `stock_reserved`, aby nie
uruchomić kaskady na nowych wierszach.

`0008_orders_pickup_date.sql` dodaje `orders.pickup_date` oraz indeks
`orders_pickup_idx (pickup_date, break_id)`. Zmiana jest addytywna:
historyczne zamówienia mają `NULL`, co oznacza „dzień złożenia”.
`pickup_date` jest używane przez raport dnia, widok kuchni „Dzisiaj”,
ważność kodu QR oraz limity „na przerwę”. Przed migracją wykonaj backup
i po starcie sprawdź `PRAGMA foreign_key_check;` oraz `PRAGMA
integrity_check;`. Migracja nie zmienia istniejących wierszy poza dodaniem
kolumny, więc liczby `orders` i `order_items` muszą pozostać identyczne.

`0009_menu_allergen_codes.sql` wprowadza ustrukturyzowane alergeny:

1. dodaje `menu_items.allergen_codes` (`TEXT NOT NULL DEFAULT ''`),
2. wypełnia ją z dawnego pola tekstowego `allergens`, mapując polskie
   i angielskie synonimy 14 alergenów UE (m.in. `mleko`/`milk`,
   `jaja`/`eggs`, `pszenica`/`wheat`, `orzech ziemne`/`peanuts`);
   nierozpoznane resztki (np. „do uzgodnienia”, literówki) są **porzucone**
   — świadoma decyzja, stare pole było wolnym tekstem testowym,
3. usuwa kolumnę `allergens` przez `DROP COLUMN`.

Uwagi i wynik testu:

- `lower()` w SQLite zmienia tylko znaki ASCII, więc backfill zawiera
  także warianty wielkich liter dla wyrażeń nie-ASCII (np. „Łosoś”,
  „Śmietana”).
- `DROP COLUMN` wymaga SQLite ≥ 3.35; kolumna nie jest częścią
  indeksu, widoku ani triggera. Test na izolowanej bazie (CLI 3.54)
  przechodzi: kolumna `allergens` znika, a `allergen_codes` zawiera
  poprawne kody (`Łosoś wędzony, mleko` → `fish,milk`).
- **Fallback**, gdyby `DROP COLUMN` zawiodł (starszy SQLite, kolumna
  wpisana w schemat): sqlx wycofuje całą migrację w transakcji, więc
  baza zostaje na `allergen_codes` bez usuniętej starej kolumny —
  aplikacja nadal działa (kod czyta wyłącznie `allergen_codes`).
  Naprawa ręczna: podnieść SQLite i wykonać migrację od nowa albo
  przebudować tabelę ręcznie (nowa tabela, kopiowanie, podmiana).
- Zmiana jest zgodna z zasadą, że migracje to wyłącznie nowe pliki;
  `0001`–`0008` pozostały nietknięte.

## 9. Ograniczenia SQLite w tym projekcie

- Jeden pisarz w danym momencie; WAL pozwala na równoległe odczyty.
- Brak wbudowanej replikacji i klastra.
- Nie używać na NFS/SMB — ryzyko korupcji.
- `INTEGER` w SQLite jest dynamiczny, ale aplikacja używa go jako i64.
- Brak pełnotekstowego wyszukiwania; filtry kont używają `LIKE`.
- Brak partycjonowania; lista audytu jest ograniczona przez `LIMIT`,
  a retencja przez wiek.
