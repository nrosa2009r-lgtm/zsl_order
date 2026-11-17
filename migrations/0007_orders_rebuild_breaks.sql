-- Przebudowa `orders`: nowe statusy, przerwy i konto usuwalne bez utraty
-- historii.
--
-- * Statusy: `ordered` → `issued`, `cancelled`, `uncollected`. Dotychczasowe
--   `new`/`preparing`/`ready` stają się `ordered`, `completed` → `issued`,
--   a `cancelled` z ustawionym `uncollected_at` → `uncollected`.
-- * `issued_at` — moment wydania; dla historycznych `completed` bierzemy
--   `paid_at`, potem `ready_at`, a na końcu `updated_at`.
-- * `user_id` może być NULL, a usunięcie konta ustawia NULL zamiast
--   blokować usunięcie (historia zamówień zostaje).
-- * `break_id` wiąże zamówienie z przerwą; usunięcie przerwy zeruje
--   powiązanie, ale nie usuwa zamówienia.
-- * `breaks` — dzienne okna wydawania (np. „Długa przerwa” 11:30–11:50).
--
-- SQLite nie potrafi zmienić CHECK ani klucza obcego przez ALTER TABLE,
-- więc tabela jest przebudowywana metodą „nowa tabela + kopia + rename”.
-- `order_items` też jest kopiowane: stara tabela wskazuje na starą
-- `orders`, a sqlx włącza `foreign_keys=ON`. Kolejność (nowe tabele,
-- kopia, usunięcie starych, rename) nie uruchamia kaskady na nowych
-- wierszach. `AUTOINCREMENT` i identyfikatory pozostają bez zmian.

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

CREATE TABLE orders_new (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id INTEGER REFERENCES users(id) ON DELETE SET NULL,
    code TEXT NOT NULL UNIQUE,
    qr_token TEXT NOT NULL UNIQUE,
    status TEXT NOT NULL DEFAULT 'ordered'
        CHECK (status IN ('ordered', 'issued', 'cancelled', 'uncollected')),
    payment_method TEXT NOT NULL CHECK (payment_method IN ('cash', 'card')),
    payment_status TEXT NOT NULL DEFAULT 'unpaid'
        CHECK (payment_status IN ('unpaid', 'paid')),
    total_cents INTEGER NOT NULL CHECK (total_cents >= 0),
    pickup_time TEXT NOT NULL DEFAULT '',
    break_id INTEGER REFERENCES breaks(id) ON DELETE SET NULL,
    paid_at INTEGER,
    paid_by INTEGER,
    ready_at INTEGER,
    issued_at INTEGER,
    uncollected_at INTEGER,
    idempotency_key TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

INSERT INTO orders_new
    (id, user_id, code, qr_token, status, payment_method, payment_status,
     total_cents, pickup_time, break_id, paid_at, paid_by, ready_at,
     issued_at, uncollected_at, idempotency_key, created_at, updated_at)
SELECT
    id,
    user_id,
    code,
    qr_token,
    CASE status
        WHEN 'new' THEN 'ordered'
        WHEN 'preparing' THEN 'ordered'
        WHEN 'ready' THEN 'ordered'
        WHEN 'completed' THEN 'issued'
        WHEN 'cancelled' THEN
            CASE WHEN uncollected_at IS NOT NULL THEN 'uncollected' ELSE 'cancelled' END
        -- Wartości nowego schematu (bezpieczne ponowne uruchomienie kopii).
        WHEN 'ordered' THEN 'ordered'
        WHEN 'issued' THEN 'issued'
        WHEN 'uncollected' THEN 'uncollected'
        ELSE 'ordered'
    END,
    payment_method,
    payment_status,
    total_cents,
    pickup_time,
    NULL,
    paid_at,
    paid_by,
    ready_at,
    CASE WHEN status = 'completed'
         THEN COALESCE(paid_at, ready_at, updated_at)
         ELSE NULL
    END,
    uncollected_at,
    idempotency_key,
    created_at,
    updated_at
FROM orders;

CREATE TABLE order_items_new (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    order_id INTEGER NOT NULL REFERENCES orders_new(id) ON DELETE CASCADE,
    menu_item_id INTEGER REFERENCES menu_items(id) ON DELETE SET NULL,
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
DROP TABLE orders;

ALTER TABLE orders_new RENAME TO orders;
ALTER TABLE order_items_new RENAME TO order_items;

CREATE INDEX IF NOT EXISTS orders_user_idx ON orders(user_id, created_at);
CREATE INDEX IF NOT EXISTS orders_status_idx ON orders(status, created_at);
CREATE UNIQUE INDEX IF NOT EXISTS orders_user_idempotency_idx
    ON orders(user_id, idempotency_key)
    WHERE idempotency_key IS NOT NULL;
CREATE INDEX IF NOT EXISTS orders_uncollected_idx ON orders(uncollected_at);
CREATE INDEX IF NOT EXISTS orders_ready_idx ON orders(status, ready_at);
CREATE INDEX IF NOT EXISTS orders_created_idx ON orders(created_at);
CREATE INDEX IF NOT EXISTS orders_idempotency_created_idx
    ON orders(created_at)
    WHERE idempotency_key IS NOT NULL;
CREATE INDEX IF NOT EXISTS orders_break_idx ON orders(break_id, created_at);

CREATE INDEX IF NOT EXISTS order_items_order_idx ON order_items(order_id);
CREATE INDEX IF NOT EXISTS order_items_menu_idx ON order_items(menu_item_id);
