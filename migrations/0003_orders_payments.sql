-- Pola potrzebne do płatności lokalnych, obsługi nieodebranych zamówień
-- i ochrony przed podwójnym wysłaniem formularza.
--
-- * `paid_at` / `paid_by` — kiedy i przez kogo kuchnia oznaczyła płatność.
--   `paid_by` nie ma twardego FK, aby audyt przetrwał usunięcie konta.
-- * `ready_at` — moment ustawienia statusu `ready`; od niego liczy się
--   limit czasu odbioru i automatyczne oznaczenie jako nieodebrane.
-- * `uncollected_at` — znacznik zamówienia nieodebranego. W bazie takie
--   zamówienie pozostaje `cancelled` (brak ryzykownej przebudowy CHECK),
--   a API prezentuje je jako status `uncollected`.
-- * `idempotency_key` — klucz generowany przez klienta; unikalny per konto.

ALTER TABLE orders ADD COLUMN paid_at INTEGER;
ALTER TABLE orders ADD COLUMN paid_by INTEGER;
ALTER TABLE orders ADD COLUMN ready_at INTEGER;
ALTER TABLE orders ADD COLUMN uncollected_at INTEGER;
ALTER TABLE orders ADD COLUMN idempotency_key TEXT;

-- Istniejące opłacone zamówienia dostają przybliżony czas płatności.
-- Nie znamy historycznego operatora, więc `paid_by` pozostaje NULL.
UPDATE orders
SET paid_at = updated_at
WHERE payment_status = 'paid' AND paid_at IS NULL;

CREATE UNIQUE INDEX IF NOT EXISTS orders_user_idempotency_idx
    ON orders(user_id, idempotency_key)
    WHERE idempotency_key IS NOT NULL;

CREATE INDEX IF NOT EXISTS orders_uncollected_idx ON orders(uncollected_at);
CREATE INDEX IF NOT EXISTS orders_ready_idx ON orders(status, ready_at);
