# Historia zmian — ZSL Order

Format oparty na [Keep a Changelog](https://keepachangelog.com/pl/1.1.0/).
Wersjonowanie zgodne z [Semantic Versioning](https://semver.org/lang/pl/).
Autorem projektu jest **natan-rosa**.

## [1.0.0] — 2026-11-13

Pierwsze wydanie produkcyjne: kompletny system zamawiania posiłków dla
stołówki szkolnej, gotowy do pilotażu w trybie lokalnym i do wdrożenia
przez Cloudflare Tunnel.

### Dodane

- Trzy niezależne panele: uczeń, kuchnia i administrator, w czystym
  HTML/CSS/JS bez frameworka i bez CDN.
- Zamówienia z cyklem życia `new → preparing → ready → completed`,
  anulowaniem przez ucznia, limitami otwartych zamówień i konfigurowalnymi
  terminami odbioru.
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
