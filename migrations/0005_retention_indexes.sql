-- Indeksy pod nocną retencję.
--
-- Czyszczenie zamówień i starych kluczy idempotencji filtruje po
-- `created_at` w całej tabeli, a nie per użytkownik. Dotychczasowe
-- indeksy zaczynają się od `user_id`/`status`, więc nie wspierały tego
-- zapytania. Oba indeksy są addytywne i nie zmieniają danych.

CREATE INDEX IF NOT EXISTS orders_created_idx ON orders(created_at);

CREATE INDEX IF NOT EXISTS orders_idempotency_created_idx
    ON orders(created_at)
    WHERE idempotency_key IS NOT NULL;
