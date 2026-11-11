-- Znacznik ręcznego zdjęcia blokady nieodebranych przez administratora.
-- Licznik blokady nadal korzysta z historii `orders.uncollected_at`, ale
-- uwzględnia tylko zamówienia nieodebrane po tej dacie. Dzięki temu akcja
-- „zdejmij blokadę” nie usuwa historii ani nie zmienia raportów.
ALTER TABLE users ADD COLUMN uncollected_block_reset_at INTEGER;
