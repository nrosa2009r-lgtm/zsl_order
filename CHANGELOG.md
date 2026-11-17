# Historia zmian — ZSL Order

Format oparty na [Keep a Changelog](https://keepachangelog.com/pl/1.1.0/).
Wersjonowanie zgodne z [Semantic Versioning](https://semver.org/lang/pl/).
Autorem projektu jest **natan-rosa**.

## [Unreleased]

### Dodane

- Uruchomienie w kontenerze: dwuetapowy `Dockerfile`, `docker-compose.yml`
  i nakładka `docker-compose.https.yml` z Caddy (TLS z wewnętrznym CA dla
  sieci lokalnej). Porty hosta: **11000 → HTTP**, **10000 → HTTPS**
  (wewnątrz kontenera nadal 3000/443). Dane trzymane w wolumenie `/data`,
  healthcheck HTTP, użytkownik UID 10001, instrukcja w `docs/DOCKER.md`.
- Log startowy podaje dokładny adres dostępowy w sieci lokalnej
  (`http://<IP>:<port>`); nasłuch wyłącznie na pętli jest jawnie
  ostrzegany.
- Opcjonalne `LOG_CLIENT_IP=1` dopisuje adres klienta do logów żądań
  (domyślnie `-` — adresy IP nie są przechowywane).
- `CLIENT_IP_HEADER` (`X-Forwarded-For` lub `X-Real-IP`) pozwala odczytać
  adres klienta za reverse proxy w sieci lokalnej; nagłówek jest
  honorowany wyłącznie od `TRUSTED_PROXIES`. Caddy w
  `docker-compose.https.yml` nadpisuje go adresem klienta, a zaufany jest
  tylko stały adres Caddy (`172.28.0.2/32`). Podrobiony nagłówek z
  niezaufanego źródła jest ignorowany.
- Dzienny stan magazynowy pozycji menu: `stock_remaining`,
  `stock_default` i `stock_date`, leniwy reset nowego dnia, rezerwacja w
  transakcji zamówienia i zwrot przy anulowaniu (tylko dzisiejsze
  zamówienia i pozycje z `order_items.stock_reserved = 1`).
- Szybka korekta stanu w panelu kuchni/admina: `-1`, `+1`, „Bez limitu”
  i endpoint `POST /api/staff/menu/:id/stock`; zmiany trafiają do audytu
  jako `menu_stock_changed`.
- Menu ucznia pokazuje „Zostało N szt.” / „Wyprzedane na dziś”, a raport
  dzienny tabelę bieżącego stanu i sprzedaży pozycji.
- Nowe ustawienia `max_item_quantity_per_break` i
  `one_active_order_per_break` (zapisywane teraz, egzekwowane po
  wdrożeniu przerw).
- Migracja `0007_orders_rebuild_breaks.sql`: statusy `ordered`, `issued`,
  `cancelled`, `uncollected`, pole `issued_at`, tabela `breaks` oraz
  `orders.user_id` dopuszczające `NULL` (`ON DELETE SET NULL`). Historia
  zamówień może przetrwać usunięcie konta; API pokazuje wtedy
  `konto usunięte`.

### Zmienione

- Deklarowane MSRV podniesione do Rusta 1.88 (wymóg `time 0.3.55`).
- Polityka haseł ma jedno źródło w `src/password_policy.rs`: uczeń 10,
  kuchnia i admin 12 znaków, limit 128. `/api/auth/me` zwraca
  `password_min_length` i gotowy komunikat, więc panel nie zaszywa już
  liczby „10” w JavaScript.
- Cykl życia zamówienia uproszczony do `ordered → issued`, z osobnymi
  statusami `cancelled` i `uncollected`; kuchnia wydaje jednym przyciskiem
  „Wydaj i opłacone”. Raport i statystyki używają pola `issued_orders`
  zamiast `completed_orders`, a opłacone nieodebrane pozostają w utargu.

### Naprawione

- Komunikat zmiany hasła nie obiecuje już „litery i cyfry”, której
  walidacja nigdy nie wymagała; pokazuje rzeczywiste wymagania (długość,
  brak loginu, brak popularnych haseł).
- Zajęta baza (`SQLITE_BUSY`/`SQLITE_LOCKED`) zwraca czytelne
  `503 unavailable` z `Retry-After: 1` zamiast `500`.

## [1.0.0] — 2026-11-13

Pierwsze wydanie produkcyjne: kompletny system zamawiania posiłków dla
stołówki szkolnej, gotowy do pilotażu w trybie lokalnym i do wdrożenia
przez Cloudflare Tunnel.

### Dodane

- Trzy niezależne panele: uczeń, kuchnia i administrator, w czystym
  HTML/CSS/JS bez frameworka i bez CDN.
- Zamówienia z cyklem życia `new → preparing → ready → completed`
  (w wersji 1.0; od kolejnej wersji `ordered → issued`), anulowaniem
  przez ucznia, limitami otwartych zamówień i konfigurowalnymi terminami
  odbioru.
- Odbiór QR: krótki kod i nieprzejrzysty token 48 hex, wyłącznie dla
  dzisiejszych zamówień `ready`, jednorazowo zużywany przez zmianę statusu.
- Skaner QR w kuchni: `BarcodeDetector`, lokalny fallback **jsQR 1.4.0**
  (Apache-2.0, vendored, bez CDN) oraz czytnik USB działający jak
  klawiatura.
- Konta bez danych osobowych: losowe loginy i hasła, pseudonimowy
  `display_code`, import klas CSV (`klasa;liczba`) i generowanie 1–100
  kont.
- Wymuszona zmiana hasła przy pierwszym logowaniu oraz reset hasła przez
  administratora.
- Jednorazowy plik CSV z hasłami startowymi budowany wyłącznie
  w przeglądarce (`Blob` + `URL.createObjectURL`), z nagłówkiem
  `klasa/grupa;login;kod konta;hasło startowe;data wygenerowania`.
- Eksport CSV metadanych kont (bez haseł) i dzienny raport sprzedaży.
- Menu z kategoriami, alergenami, cenami i dostępnością; statystyki
  i wykresy aktywności 7/14/30/90 dni.
- Wbudowane kopie zapasowe SQLite z weryfikacją `integrity_check`,
  retencją i timerami systemd.
- Nocna retencja zamówień i audytu porcjami, uruchamiana po backupie.
- Tryby wdrożenia `local` i `tunnel`, konfiguracja przez `.env`,
  `origin_guard` i obsługa zaufanego proxy.
- Tokeny dostępowe JWT (15 min) powiązane z wierszem sesji, dzięki czemu
  logout, zmiana hasła i dezaktywacja konta unieważniają token.
- Strukturalne logi JSON na stdout/journald oraz dziennik audytu zdarzeń
  bezpieczeństwa.
- Workflow CI: `cargo fmt`, testy, `clippy -D warnings`, smoke testy,
  `cargo audit` i `cargo deny`.

### Bezpieczeństwo

- Hasła wyłącznie jako Argon2id z losową solą; brak haseł w bazie,
  audycie, logach i odpowiedziach innych endpointów.
- Rate limiting logowania per konto i per IP, blokada po serii błędów
  oraz niezależny limiter odbioru QR.
- Sesje w ciasteczkach `HttpOnly` + `SameSite=Strict` + `Secure`
  (w HTTPS); w bazie wyłącznie SHA-256 tokenu.
- CSRF, kontrola `Origin`/`Host`, CSP `script-src 'self'`, nagłówki
  `no-store` dla odpowiedzi z sekretami.
- `admin_start.txt` z uprawnieniami `0600`, nie jest serwowany przez HTTP
  i jest usuwany po pierwszej zmianie hasła.
- Uprawnienia bazy, WAL i `jwt.key` `0600`, katalog stanu `0700`,
  zaostrzona jednostka systemd (`ProtectSystem`, `NoNewPrivileges`,
  `CapabilityBoundingSet=` itd.).
- Polityka haseł: minimum 12 znaków (15 dla administratora), zakaz
  powtarzania loginu i popularnych haseł.
- Generator haseł wyklucza znaki mylące (`0/O`, `1/l/I`) i specjalne
  (`= + - @`); login i hasło nie zaczynają się od znaku specjalnego.

### Zmienione

- Spójna nazwa wdrożeniowa `zsl-order`: usługa, użytkownik, grupa,
  `/opt/zsl-order`, `/var/lib/zsl-order`, `/etc/zsl-order`.
- Retencja uruchamiana o 04:00, po backupie o 03:15.
- Dokumentacja polityki haseł dopasowana do faktycznych wymagań.

### Dokumentacja

- Pełna instrukcja obsługi dla administratora, kuchni i uczniów oraz
  jednostronicowe ściągawki do wydruku.
- Checklista wdrożenia z podziałem na pilotaż na Dellu, produkcję dla
  całej szkoły i włączenie tunelu Cloudflare (zgody, backup, ZAP, test
  obciążeniowy, plan papierowy, wyłącznik awaryjny, zgłaszanie naruszeń
  w 72 h).
- Opis API, architektury, bazy danych, bezpieczeństwa, prywatności/RODO
  i wdrożenia.
- Rejestr licencji: MIT dla projektu oraz Apache-2.0 dla jsQR, wraz
  z sumami SHA-256 i źródłem pobrania.

### Znane ograniczenia

- SQLite i jedna instancja aplikacji — rozwiązanie dla jednej stołówki.
- Brak płatności online i samoobsługowego resetu hasła (celowo).
- Kamera QR wymaga HTTPS albo `localhost`; w sieci lokalnej zalecany
  czytnik USB.
