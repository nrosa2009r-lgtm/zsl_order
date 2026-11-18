-- Usunięcie alergenów z menu.
--
-- System nie przechowuje informacji o alergenach — szkoły podają je
-- klientom poza systemem (np. tablica przy wydawaniu; patrz
-- docs/INSTRUKCJA.md i docs/CHECKLISTA-WDROZENIA.md). Migracja usuwa
-- obie kolumny alergenów, które mogą występować w bazie:
--
-- * `allergens`       — pole tekstowe wprowadzone w 0001,
-- * `allergen_codes`  — pole z wcześniejszej wersji 0009, jeśli baza
--                       zdążyła zastosować tamtą wersję migracji.
--
-- Żadnych wartości alergenów nie zgadujemy ani nie przenosimy.
--
-- SQLite nie ma `DROP COLUMN IF EXISTS`, a sqlx wykonuje migrację w jednej
-- transakcji z włączonym `foreign_keys=ON` (nie da się go wyłączyć w
-- trakcie), więc kolumny znikają przez przebudowę tabeli: kopiowane są
-- wyłącznie kolumny spoza alergenów, co działa niezależnie od tego, które
-- kolumny akurat istnieją.
--
-- `order_items` jest przebudowywane razem z `menu_items` (wzór z 0007):
-- stara tabela wskazuje na starą `menu_items`, więc samo usunięcie
-- `menu_items` uruchomiłoby `ON DELETE SET NULL` i wyzerowałoby
-- `order_items.menu_item_id` — stracilibyśmy powiązania zamówień
-- z pozycjami. Kolejność (nowe tabele, kopia, usunięcie starych, rename)
-- nie uruchamia kaskady na nowych wierszach: `order_items_new` wskazuje
-- na `menu_items_new`, a `ALTER TABLE ... RENAME` przepisuje klucz obcy
-- na nazwę `menu_items`. Identyfikatory i `AUTOINCREMENT` pozostają
-- bez zmian, a usunięte indeksy są odtwarzane na końcu.

CREATE TABLE menu_items_new (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    price_cents INTEGER NOT NULL CHECK (price_cents >= 0),
    category TEXT NOT NULL DEFAULT 'Dania',
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

INSERT INTO menu_items_new
    (id, name, description, price_cents, category, available, sort_order,
     created_at, updated_at, stock_remaining, stock_default, stock_date)
SELECT id, name, description, price_cents, category, available, sort_order,
       created_at, updated_at, stock_remaining, stock_default, stock_date
FROM menu_items;

CREATE TABLE order_items_new (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    order_id INTEGER NOT NULL REFERENCES orders(id) ON DELETE CASCADE,
    menu_item_id INTEGER REFERENCES menu_items_new(id) ON DELETE SET NULL,
    name TEXT NOT NULL,
    price_cents INTEGER NOT NULL CHECK (price_cents >= 0),
    quantity INTEGER NOT NULL CHECK (quantity > 0),
    stock_reserved INTEGER NOT NULL DEFAULT 0 CHECK (stock_reserved IN (0, 1))
);

INSERT INTO order_items_new
    (id, order_id, menu_item_id, name, price_cents, quantity, stock_reserved)
SELECT id, order_id, menu_item_id, name, price_cents, quantity, stock_reserved
FROM order_items;

DROP TABLE order_items;
DROP TABLE menu_items;

ALTER TABLE menu_items_new RENAME TO menu_items;
ALTER TABLE order_items_new RENAME TO order_items;

CREATE INDEX IF NOT EXISTS menu_available_idx ON menu_items(available, sort_order);
CREATE INDEX IF NOT EXISTS menu_items_stock_date_idx ON menu_items(stock_date);
CREATE INDEX IF NOT EXISTS order_items_order_idx ON order_items(order_id);
CREATE INDEX IF NOT EXISTS order_items_menu_idx ON order_items(menu_item_id);
