-- Dzienny stan magazynowy pozycji menu.
--
-- * `stock_remaining` — ile sztuk zostało na dzień `stock_date`.
--   NULL oznacza „bez limitu” (pozycja zawsze dostępna).
-- * `stock_default` — opcjonalny stan, do którego pozycja wraca przy
--   pierwszym odczycie nowego dnia. NULL oznacza brak automatycznego
--   resetu: stan ustawiony ręcznie obowiązuje do następnej zmiany.
-- * `stock_date` — dzień (Europe/Warsaw, `RRRR-MM-DD`), którego dotyczy
--   `stock_remaining`. Reset jest leniwy: wykonywany w transakcji przy
--   pierwszym odczycie lub zamówieniu nowego dnia, bez zadania w tle.
--
-- CHECK blokuje wartości ujemne nawet w razie błędu w kodzie. Kolumny
-- są dodawane przez ALTER TABLE, więc istniejące pozycje zachowują dane
-- i dostają NULL (bez limitu).

ALTER TABLE menu_items ADD COLUMN stock_remaining INTEGER
    CHECK (stock_remaining IS NULL OR stock_remaining >= 0);

ALTER TABLE menu_items ADD COLUMN stock_default INTEGER
    CHECK (stock_default IS NULL OR stock_default >= 0);

ALTER TABLE menu_items ADD COLUMN stock_date TEXT;

-- Czy pozycja zamówienia zarezerwowała limitowany stan? Dzięki temu
-- anulowanie zwraca sztuki tylko tym pozycjom, które faktycznie je
-- zabrały (a nie tym, którym limit ustawiono już po złożeniu zamówienia).
ALTER TABLE order_items ADD COLUMN stock_reserved INTEGER NOT NULL DEFAULT 0
    CHECK (stock_reserved IN (0, 1));

CREATE INDEX IF NOT EXISTS menu_items_stock_date_idx ON menu_items(stock_date);
