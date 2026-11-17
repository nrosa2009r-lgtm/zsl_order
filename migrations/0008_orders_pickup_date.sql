-- Dzień odbioru zamówienia.
--
-- * `pickup_date` (`RRRR-MM-DD`, strefa szkoły) pozwala zamówić posiłek
--   z wyprzedzeniem, gdy `order_days_ahead > 0`.
-- * `NULL` oznacza zamówienie historyczne: jego dzień odbioru to dzień
--   złożenia (`created_at`). Dzięki temu migracja nie musi przeliczać
--   stref czasowych dla starych wierszy, a kod ma jednoznaczne fallback.
-- * Stan magazynowy jest nadal dzienny i kontrolowany wyłącznie dla
--   zamówień na dziś; zamówienia przyszłe nie rezerwują stanu.
--
-- Indeks przyspiesza raport dnia, widok kuchni „Dzisiaj” oraz liczenie
-- limitów „na przerwę”.

ALTER TABLE orders ADD COLUMN pickup_date TEXT
    CHECK (pickup_date IS NULL OR pickup_date GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]');

CREATE INDEX IF NOT EXISTS orders_pickup_idx ON orders(pickup_date, break_id);
