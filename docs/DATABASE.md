# Baza danych — ZSL Order

Aplikacja używa SQLite w trybie WAL. Schemat jest tworzony idempotentnie
przy starcie (`CREATE TABLE IF NOT EXISTS`, `CREATE INDEX IF NOT EXISTS`).
Nie ma zewnętrznego serwera bazy ani frameworka migracji.

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
- `orders.user_id` → `users.id` `ON DELETE RESTRICT`
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
    created_at INTEGER NOT NULL,
    last_login_at INTEGER
);
CREATE INDEX IF NOT EXISTS users_role_idx ON users(role);
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
| `created_at` | Unix UTC |
| `last_login_at` | Unix UTC, może być `NULL` |

Brak pól osobowych: imienia, nazwiska, e-maila, telefonu, klasy itp.

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
| `token_hash` | SHA-256 tokenu sesji (hex); jawny token tylko w ciasteczku |
| `user_id` | właściciel sesji |
| `csrf_token` | losowy token CSRF (jawny, potrzebny do porównania) |
| `created_at` | Unix UTC |
| `expires_at` | Unix UTC |
| `last_seen_at` | Unix UTC; odświeżane maks. co 5 min |

Wygasłe sesje są usuwane przy logowaniu i starcie aplikacji.

### `menu_items`

```sql
CREATE TABLE IF NOT EXISTS menu_items (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    price_cents INTEGER NOT NULL CHECK (price_cents >= 0),
    category TEXT NOT NULL DEFAULT 'Dania',
    allergens TEXT NOT NULL DEFAULT '',
    available INTEGER NOT NULL DEFAULT 1,
    sort_order INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS menu_available_idx ON menu_items(available, sort_order);
```

- `available = 0` to miękkie ukrycie; pozycja pozostaje w historii.
- `sort_order` steruje kolejnością; mniejsza wartość = wyżej.
- Pozycje są snapshotowane do `order_items` w momencie zamówienia.

### `orders`

```sql
CREATE TABLE IF NOT EXISTS orders (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
    code TEXT NOT NULL UNIQUE,
    qr_token TEXT NOT NULL UNIQUE,
    status TEXT NOT NULL DEFAULT 'new'
        CHECK (status IN ('new','preparing','ready','completed','cancelled')),
    payment_method TEXT NOT NULL CHECK (payment_method IN ('cash','card')),
    payment_status TEXT NOT NULL DEFAULT 'unpaid'
        CHECK (payment_status IN ('unpaid','paid')),
    total_cents INTEGER NOT NULL CHECK (total_cents >= 0),
    pickup_time TEXT NOT NULL DEFAULT '',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS orders_user_idx ON orders(user_id, created_at);
CREATE INDEX IF NOT EXISTS orders_status_idx ON orders(status, created_at);
```

| Kolumna | Opis |
| --- | --- |
| `user_id` | właściciel (pseudonimowe konto) |
| `code` | 6 wielkich liter, unikalny kod odbioru |
| `qr_token` | 48 znaków hex (24 B), unikalny token QR |
| `status` | `new`, `preparing`, `ready`, `completed`, `cancelled` |
| `payment_method` | `cash`, `card` |
| `payment_status` | `unpaid`, `paid` |
| `total_cents` | suma policzona po stronie serwera |
| `pickup_time` | `""` lub `HH:MM` (godzina lokalna) |
| `created_at`, `updated_at` | Unix UTC |

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
    quantity INTEGER NOT NULL CHECK (quantity > 0)
);
CREATE INDEX IF NOT EXISTS order_items_order_idx ON order_items(order_id);
```

- `name` i `price_cents` to **snapshot** z momentu zakupu.
- `menu_item_id` może stać się `NULL`, jeśli pozycja menu zostanie
  kiedykolwiek fizycznie usunięta (aplikacja tylko ukrywa).
- `quantity` 1–10 (walidacja aplikacji; baza wymaga > 0).

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
| `orders_user_idx` | `orders` | `user_id, created_at` | historia ucznia |
| `orders_status_idx` | `orders` | `status, created_at` | kolejka kuchni |
| `order_items_order_idx` | `order_items` | `order_id` | składanie zamówień |
| `audit_created_idx` | `audit_log` | `created_at` | retencja, lista |

Unikalne ograniczenia (`UNIQUE`) tworzą dodatkowe indeksy na `users.login`,
`users.display_code`, `orders.code`, `orders.qr_token`.

## 4. Seed i bootstrap

Przy każdym starcie:

1. Tworzenie tabel/indeksów (idempotentne).
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
WHERE status IN ('new','preparing','ready')
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

## 6. Retencja i porządki

Automatyczne:

- sesje wygasłe — przy logowaniu i starcie,
- audyt starszy niż 90 dni — przy starcie.

Ręczne (zalecane, zgodnie z polityką szkoły):

- zamówienia starsze niż rok szkolny: usuń lub zanonimizuj,
- konta nieaktywne po zakończeniu roku: dezaktywuj, a po okresie retencji
  usuń (pamiętaj o `orders.user_id ON DELETE RESTRICT`).

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

Nie usuwaj użytkownika, dopóki ma zamówienia (`RESTRICT`). Najpierw
rozstrzygnij historię zamówień.

## 7. Backup i integralność

- Backup: `sqlite3 baza.db ".backup 'kopia.db'"` lub `VACUUM INTO`.
- Nie kopiuj `baza.db` przez `cp` podczas pracy — WAL może być niespójny.
- Po odtworzeniu usuń `baza.db-wal` i `baza.db-shm`.
- Sprawdzenie integralności:

```bash
sqlite3 baza.db "PRAGMA integrity_check;"
sqlite3 baza.db "PRAGMA foreign_key_check;"
```

- Backup zawiera hash haseł, tokeny sesji i tokeny QR — traktuj jak dane
  wrażliwe.

## 8. Migracje

Brak frameworka migracji. Zasady:

1. Zawsze backup przed zmianą schematu.
2. Testuj migrację na kopii bazy.
3. Preferuj zmiany addytywne (`ALTER TABLE ... ADD COLUMN`,
   `CREATE INDEX IF NOT EXISTS`).
4. SQLite ma ograniczone `ALTER TABLE`; przebudowa tabeli wymaga
   utworzenia nowej, kopiowania danych i podmiany w transakcji.
5. Aplikacja przy starcie nie usuwa ani nie modyfikuje istniejących
   danych; seed menu działa tylko na pustej tabeli.

## 9. Ograniczenia SQLite w tym projekcie

- Jeden pisarz w danym momencie; WAL pozwala na równoległe odczyty.
- Brak wbudowanej replikacji i klastra.
- Nie używać na NFS/SMB — ryzyko korupcji.
- `INTEGER` w SQLite jest dynamiczny, ale aplikacja używa go jako i64.
- Brak pełnotekstowego wyszukiwania; filtry kont używają `LIKE`.
- Brak partycjonowania; lista audytu jest ograniczona przez `LIMIT`,
  a retencja przez wiek.
