# Procedura wydania — ZSL Order

Dokument dla osoby publikującej nową wersję. Opisuje weryfikację,
tagowanie, Release na GitHubie i wycofanie wydania. Historia zmian
znajduje się w [`CHANGELOG.md`](../CHANGELOG.md).

## 1. Zasady wersjonowania

- Wersja w `Cargo.toml` (`version`) jest jedynym źródłem prawdy.
- Stosujemy SemVer: `MAJOR.MINOR.PATCH`.
  - `MAJOR` — niekompatybilna zmiana API lub konfiguracji,
  - `MINOR` — nowe funkcje zachowujące kompatybilność,
  - `PATCH` — poprawki błędów i bezpieczeństwa.
- Tag ma postać `vMAJOR.MINOR.PATCH`, np. `v1.0.0`.
- Migracje bazy są addytywne; wydanie nie może zmieniać treści
  istniejących plików w `migrations/` (zmieniłoby to sumy kontrolne
  `_sqlx_migrations`).

## 2. Weryfikacja przed wydaniem

Na czystym checkoutcie (`git status` bez zmian):

```bash
cargo fmt --all --check
cargo test --locked
cargo clippy --all-targets -- -D warnings
cargo build --release --locked
cargo audit
cargo deny check
scripts/test-migrations.sh
scripts/smoke-modes.sh
scripts/smoke-hardening.sh
scripts/smoke-orders.sh
scripts/smoke-backup.sh
scripts/smoke-logs.sh
```

Dodatkowo:

- [ ] Test odtworzenia backupu na drugiej maszynie (`integrity_check` = `ok`).
- [ ] Sprawdzenie, że `git ls-files` nie zawiera `baza.db`, `jwt.key`,
      `admin_start.txt`, `config.json`, `.env` ani kopii zapasowych.
- [ ] Przegląd diffu pod kątem sekretów, adresów IP i danych osobowych.
- [ ] Wpis w `CHANGELOG.md` z datą i zakresem zmian.
- [ ] Zgody/dokumentacja szkoły zaktualizowane, jeśli zmienił się zakres
      danych lub retencja (`docs/CHECKLISTA-WDROZENIA.md`).

## 3. Przygotowanie wersji

1. Zaktualizuj `version` w `Cargo.toml` i uruchom `cargo build --locked`,
   aby odświeżyć `Cargo.lock`.
2. Dopisz sekcję w `CHANGELOG.md` (data, kategorie: Dodane, Zmienione,
   Naprawione, Bezpieczeństwo).
3. Zacommituj zmiany, np.:

   ```bash
   git add Cargo.toml Cargo.lock CHANGELOG.md
   git commit -m "Wydanie 1.0.0"
   ```

4. Wypchnij `master` (bez `--force`):

   ```bash
   git push origin master
   ```

## 4. Tag i Release

```bash
git tag -a v1.0.0 -m "ZSL Order 1.0.0"
git push origin v1.0.0
```

Na GitHubie: **Releases → Draft a new release** → wybierz tag `v1.0.0`,
tytuł `ZSL Order 1.0.0` i wklej sekcję `1.0.0` z `CHANGELOG.md`.
Nie dodawaj do opisu danych uczniów, adresów IP ani sekretów.

## 5. Wdrożenie po wydaniu

- Aktualizacja według [`DEPLOYMENT.md`](DEPLOYMENT.md) §9: backup,
  zatrzymanie usługi, podmiana binarki i `frontend/`, start, `healthz`.
- Sprawdź `_sqlx_migrations` i `PRAGMA integrity_check`.
- Zrób wpis w rejestrze wdrożeń: wersja, data, osoba, wynik testów.

## 6. Wycofanie wydania (rollback)

1. Zatrzymaj usługę: `sudo systemctl stop zsl-order`.
2. Przywróć poprzednią binarkę i `frontend/` z artefaktu wydania.
3. W razie niekompatybilnej migracji przywróć bazę z backupu wykonanego
   przed aktualizacją (`backup --verify`).
4. Uruchom usługę i sprawdź `healthz`, logowanie oraz odczyt zamówień.
5. Oznacz wadliwe wydanie jako „yanked” w Release i opisz powód.

## 7. Wydanie awaryjne (bezpieczeństwo)

Przy podatności:

- przygotuj poprawkę jako `PATCH`, bez zbędnych zmian funkcjonalnych,
- przećwicz scenariusz unieważnienia sesji i rotacji `jwt.key`,
- nie publikuj szczegółów wykorzystania przed wdrożeniem poprawki,
- po wdrożeniu opisz problem w `CHANGELOG.md` i, jeśli wymagane,
  przekaż zgłoszenie przez szkołę zgodnie z procedurą 72 h.
