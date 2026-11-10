-- Kolumna `class_group` dla kont uczniów.
--
-- Do tej pory klasa/grupa była tylko częścią loginu (`s-1a-...`) i kodu
-- konta (`UCZ-1A-...`). Osobna kolumna pozwala filtrować i raportować bez
-- parsowania loginu. Wartość jest pseudonimem klasy, nie danymi osobowymi.
--
-- Backfill dotyczy wyłącznie kont zaimportowanych z CSV, których login ma
-- format `s-<klasa>-<losowe znaki>`. Zwykłe konta `s-<losowe znaki>`
-- pozostają z NULL.

ALTER TABLE users ADD COLUMN class_group TEXT;

UPDATE users
SET class_group = upper(substr(login, 3, instr(substr(login, 3), '-') - 1))
WHERE role = 'student'
  AND login LIKE 's-%-%'
  AND instr(substr(login, 3), '-') > 0;

CREATE INDEX IF NOT EXISTS users_class_group_idx ON users(class_group);
