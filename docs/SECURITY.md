# Bezpieczeństwo — ZSL Order

Dokument opisuje model zagrożeń, mechanizmy ochrony, konfigurację
produkcyjną, ograniczenia oraz checklistę wdrożeniową. Nie jest
certyfikatem bezpieczeństwa — to opis stanu implementacji i wytyczne.

## 1. Model zagrożeń (skrót)

Chronimy przede wszystkim:

| Zagrożenie | Przykład | Główne środki |
| --- | --- | --- |
| Przejęcie konta | zgadywanie hasła, credential stuffing | Argon2id, rate limiting, losowe hasła, wymuszona zmiana |
| Kradzież sesji | XSS, wyciek ciasteczka, przechwycenie HTTP | `HttpOnly`, `SameSite=Strict`, `Secure`, CSP, TLS, tokeny tylko jako hash |
| CSRF | obca strona wysyła żądanie w imieniu zalogowanego | token CSRF + `Origin`/`Sec-Fetch-Site` |
| Brak autoryzacji | uczeń sięga po panel admina | guardy ról na poddrzewach tras |
| IDOR | uczeń czyta QR/cudze zamówienie po ID | zapytania zawsze z `user_id` właściciela |
| SQL injection | manipulacja parametrami | wyłącznie zapytania parametryzowane |
| XSS | złośliwa nazwa w menu | `textContent`, brak `innerHTML`, CSP bez inline |
| Clickjacking | osadzenie panelu w iframe | `frame-ancestors 'none'`, `X-Frame-Options: DENY` |
| Wyciek danych z bazy | kopia `baza.db` | hash haseł, hash tokenów sesji, minimalizacja danych, uprawnienia pliku |
| Nadużycie QR | publiczne udostępnienie kodu | QR to bearer token; wymaga chronionego kanału i nie zawiera PII |
| Przeciążenie | ogromne ciało żądania, spam logowania | limit 64 KiB, limiter, limity list i kont |
| Błąd ujawniający szczegóły | stack trace/SQL w odpowiedzi | `AppError` z generycznym komunikatem, log tylko po stronie serwera |

Poza zakresem: ataki na host/DB, złośliwy administrator, kompromitacja
przeglądarki użytkownika, fizyczny dostęp do serwera, ataki
denial-of-service na warstwie sieci.

## 2. Uwierzytelnianie i hasła

- **Argon2id** (`argon2` 0.5, `Argon2::default()`): domyślne parametry
  (19 MiB pamięci, 2 iteracje, 1 wątek), losowa sól per hasło, wersja 0x13.
- Hasła generowane losowo:
  - konta uczniów/kuchni/admina: 14 znaków,
  - startowy administrator: 18 znaków,
  - alfabet bez znaków mylonych (`0/O`, `1/l/I`), gwarantowana wielka
    litera i cyfra, losowa kolejność (Fisher–Yates).
- Hasła nigdy nie są przechowywane jawnie i nigdy nie są logowane.
- Startowe hasło admina jest wypisywane w konsoli i zapisywane do
  `admin_start.txt` z uprawnieniami `0600`; plik jest usuwany po zmianie
  hasła.
- Wymuszona zmiana hasła (`must_change_password`) blokuje całe API poza
  `me`, `logout`, `change-password`.
- Polityka nowego hasła: 10–128 znaków, litera ASCII + cyfra, różne od
  obecnego.
- Brak ujawniania, czy login istnieje: dla nieistniejącego konta wykonywane
  jest weryfikacja względem `dummy_hash`, aby wyrównać czas odpowiedzi.
- Rate limiting logowania (w pamięci):
  - 10 nieudanych prób na login w 10 min → blokada 15 min,
  - 80 nieudanych prób na IP w 10 min → blokada 15 min,
  - sukces czyści liczniki dla danego IP/loginu.
- Brak MFA — patrz [ograniczenia](#10-ograniczenia-i-residualne-ryzyko).

## 3. Sesje

- Token: 32 losowe bajty (`rand`) → 64 znaki hex.
- W bazie przechowywany jest **wyłącznie SHA-256 tokenu**; kopia bazy nie
  pozwala odtworzyć ciasteczek.
- Ciasteczko:
  - nazwa `zsl_session`,
  - `Path=/`,
  - `HttpOnly`,
  - `SameSite=Strict`,
  - `Max-Age = session_hours * 3600` (domyślnie 12 h),
  - `Secure` tylko gdy `secure_cookies = true`.
- Brak `Domain` — ciasteczko jest host-only.
- Sesja zawiera `csrf_token`, `created_at`, `expires_at`, `last_seen_at`.
- `last_seen_at` jest odświeżane maks. raz na 5 minut (mniej zapisów).
- Wylogowanie usuwa sesję. Zmiana hasła usuwa wszystkie pozostałe sesje.
  Dezaktywacja konta lub zmiana roli usuwa wszystkie sesje.
- Wygasłe sesje są czyszczone przy logowaniu i starcie.
- Brak „remember me” i brak sesji trwałych poza `session_hours`.

## 4. CSRF i kontrola pochodzenia

Dla metod `POST`/`PUT`/`PATCH`/`DELETE` na trasach chronionych:

1. Wymagany nagłówek `X-CSRF-Token` zgodny z tokenem sesji.
2. Porównanie w czasie stałym (`constant_time_eq`).
3. Brak/niezgodność → `403 forbidden`.

Dodatkowo `origin_guard` (wszystkie metody zmieniające stan, także login):

- `Sec-Fetch-Site: cross-site` → `403`,
- jeśli jest `Origin`, to jego host musi być równy `Host`; `null`, pusty
  lub inny host → `403`.

Żądania bez `Origin` są dozwolone (np. `curl`), ale nadal obowiązuje CSRF.
Nagłówek `Origin` nie jest jedynym mechanizmem — to warstwa dodatkowa.

## 5. Autoryzacja

- Role: `student`, `kitchen`, `admin`; przechowywane w `users.role`
  z ograniczeniem `CHECK`.
- Guardy:
  - `require_student` — tylko `student`,
  - `require_staff` — `kitchen` lub `admin`,
  - `require_admin` — tylko `admin`.
- Guardy są nakładane na całe poddrzewa `/api/student`, `/api/staff`,
  `/api/admin`.
- Zapytania o zamówienia ucznia zawsze zawierają `user_id` z sesji:
  `GET /api/student/orders/:id/...` nie pozwala na dostęp po samym ID.
- Anulowanie sprawdza właściciela i status `new`.
- Kuchnia ma dodatkowe ograniczenia statusów/płatności; administrator
  może więcej, ale nadal w ramach dozwolonych wartości.
- Administrator nie może odebrać uprawnień własnemu kontu (dezaktywacja
  lub zmiana roli) — ochrona przed samozablokowaniem.
- Reset hasła nie jest dostępny dla własnego konta (własne hasło zmienia
  się przez `change-password`).

## 6. Nagłówki bezpieczeństwa

Dla wszystkich odpowiedzi:

```http
Content-Security-Policy: default-src 'self'; script-src 'self'; style-src 'self';
  img-src 'self' data: blob:; font-src 'self'; connect-src 'self';
  object-src 'none'; base-uri 'none'; form-action 'self';
  frame-ancestors 'none'
X-Content-Type-Options: nosniff
X-Frame-Options: DENY
Referrer-Policy: no-referrer
Permissions-Policy: camera=(self), microphone=(), geolocation=()
Cross-Origin-Opener-Policy: same-origin
Cross-Origin-Resource-Policy: same-origin
```

Gdy `secure_cookies = true`, dodatkowo:

```http
Strict-Transport-Security: max-age=31536000; includeSubDomains
Content-Security-Policy: ...; upgrade-insecure-requests
```

Dla `/api/*`:

```http
Cache-Control: no-store
```

CSP nie dopuszcza skryptów inline, `eval`, obcych domen ani ramek.
`camera=(self)` jest potrzebne wyłącznie do skanera QR w panelu kuchni.
`microphone` i `geolocation` są wyłączone.

## 7. Walidacja, SQL i XSS

- Wszystkie zapytania SQL są parametryzowane (`sqlx::query`, `bind`).
  Dynamiczne filtry (lista ID, filtry kont) używają `QueryBuilder` z
  bindowaniem, nie konkatenacji stringów.
- Wejścia są przycinane i walidowane po stronie serwera (długości,
  zakresy, formaty, enumy). Frontend ma dodatkowe ograniczenia, ale nie
  jest zaufany.
- Kwoty są liczone po stronie serwera na podstawie cen z bazy; klient nie
  przesyła ceny.
- Brak `unsafe` w kodzie aplikacji.
- Frontend buduje DOM przez `textContent`/`createElement`; brak
  `innerHTML`, `insertAdjacentHTML`, `eval`, `new Function`,
  `document.write`.
- Błędy wewnętrzne są logowane na stderr, a klient dostaje generyczny
  komunikat `internal_error` bez SQL/ścieżek/stack trace.
- Limit ciała żądania 64 KiB; limit liczby pozycji, kont, list i długości
  pól.

## 8. Kody QR i odbiór

- Token QR: 24 losowe bajty → 48 znaków hex (192 bity).
- Zawartość QR: `ZSL:<token>` — bez loginu, kodu ucznia i innych danych.
- Krótki kod: 6 liter z 24-znakowego alfabetu (bez cyfr i znaków
  mylonych), unikalny w bazie; ok. 28 bitów entropii.
- Krótki kod **nie jest publicznym sekretem** — jego użycie wymaga
  zalogowanego pracownika (`kitchen`/`admin`) i przechodzi przez CSRF.
- QR jest generowany tylko dla właściciela zamówienia; inny uczeń dostaje
  `404`.
- Anulowane zamówienie nie generuje QR (`409`).
- Pickup nie zmienia statusu; wydanie jest osobną akcją personelu.
- Token QR jest przechowywany jawnie w bazie (konieczny do generowania
  obrazu). Ochrona bazy = ochrona kodów odbioru.

## 9. Audyt i logowanie

- Tabela `audit_log`: `actor_id`, `action`, `details`, `created_at`.
- Zdarzenia m.in.: `login`, `logout`, `password_changed`,
  `password_change_failed`, `password_reset`, `users_generated`,
  `user_updated`, `order_created`, `order_cancelled_by_student`,
  `order_updated`, `pickup_lookup`, `menu_item_created`,
  `menu_item_updated`, `menu_item_hidden`, `settings_updated`,
  `admin_seeded`.
- `details` zawierają identyfikatory techniczne i statusy, nie imiona ani
  dane kontaktowe.
- Retencja: wpisy starsze niż 90 dni są usuwane przy starcie.
- Aplikacja nie prowadzi access logów HTTP (brak `TraceLayer`); logi
  błędów trafiają na stderr (systemd journal). Nie loguj ciasteczek,
  haseł ani tokenów.
- Audyt nie jest niezmienny — administrator z dostępem do bazy może go
  modyfikować. Dla silniejszego reżimu należy wysyłać kopię do
  zewnętrznego, append-only systemu.

## 10. Ograniczenia i residualne ryzyko

| Obszar | Ograniczenie | Rekomendacja |
| --- | --- | --- |
| MFA | brak dla adminów | silne unikalne hasła, ograniczenie dostępu sieciowego, ewentualnie MFA przed wdrożeniem publicznym |
| Rate limiting | w pamięci, per proces; za proxy widzi IP proxy | uruchamiać bezpośrednio za firewallem lub dodać zaufane `X-Forwarded-For` w kodzie; nie polegać wyłącznie na limiterze |
| Fallback nasłuchu | przy zajętym porcie aplikacja próbuje `0.0.0.0:3000` | zapewnić wolny port; w produkcji zweryfikować interfejs nasłuchu |
| Domyślna konfiguracja | `ip = 0.0.0.0`, `secure_cookies = false` | zawsze ustawiać `127.0.0.1` + HTTPS + `secure_cookies = true` |
| HTTPS | aplikacja nie terminuje TLS | obowiązkowy reverse proxy (Nginx/Caddy) z ważnym certyfikatem |
| SQLite | jeden plik, brak replikacji | backup + monitoring; nie udostępniać przez NFS |
| Retencja zamówień | brak automatycznego usuwania | ustalić politykę i okresowo anonymizować/usuwać |
| Migracje | brak frameworka migracji | testować zmiany schematu na kopii; robić backup przed aktualizacją |
| Audyt | edytowalny przez admina DB | zewnętrzna kopia append-only |
| QR | bearer token w bazie i na ekranie | nie publikować QR; traktować jak bilet |
| Krótki kod | ~28 bitów | tylko personel; nie używać jako publicznego linku |
| Zależności | brak automatycznego audytu w CI | uruchamiać `cargo audit`, aktualizować zależności |
| Logi HTTP | brak | dodać na poziomie reverse proxy, z retencją i bez danych wrażliwych |
| Plik `admin_start.txt` | może pozostać, jeśli hasło nie zostanie zmienione | jak najszybciej zmienić hasło; plik ma `0600` |
| Wszyscy admini nieaktywni | bootstrap tworzy konto tylko przy zerze rekordów `admin` | nie dezaktywować ostatniego admina; procedura awaryjna przez DB |
| CSRF dla klientów API | wymaga nagłówka; klienci bez niego dostaną 403 | udokumentowane w [API.md](API.md) |

## 11. Checklista wdrożeniowa

### Konfiguracja

- [ ] `ip = "127.0.0.1"` (za reverse proxy) lub adres w zaufanej sieci.
- [ ] `secure_cookies = true` i działający HTTPS.
- [ ] `session_hours` zgodne z polityką (domyślnie 12 h).
- [ ] `max_connections` rozsądne (8–16).
- [ ] `config.json` i `baza.db` z uprawnieniami `0600`/`0640`, właściciel
      = użytkownik usługi.
- [ ] `admin_start.txt` odebrany i usunięty po pierwszej zmianie hasła.

### Host

- [ ] Dedykowany użytkownik systemowy, bez logowania.
- [ ] Firewall: dostęp do portu aplikacji tylko z reverse proxy.
- [ ] systemd z `NoNewPrivileges`, `ProtectSystem`, `ProtectHome`,
      `PrivateTmp`, `UMask=0077`.
- [ ] Aktualny system i zależności; `cargo audit`.
- [ ] Synchronizacja czasu (NTP) — poprawne znaczniki czasu i TLS.

### Aplikacja

- [ ] Hasło startowe admina zmienione; `admin_start.txt` nie istnieje.
- [ ] Konta uczniów/kuchni wygenerowane; hasła przekazane bezpiecznie.
- [ ] Zbędne konta admina zminimalizowane.
- [ ] Menu i ustawienia skonfigurowane; `ordering_enabled` zgodne z celem.
- [ ] Sprawdzone nagłówki (`curl -I`): CSP, `X-Frame-Options`,
      `nosniff`, `no-referrer`, HSTS przy HTTPS.
- [ ] Sprawdzone działanie CSRF (żądanie bez nagłówka → 403).
- [ ] Sprawdzone role (uczeń → `/api/admin/*` = 403).
- [ ] Sprawdzone logowanie, wymuszona zmiana hasła, reset hasła.
- [ ] Sprawdzone składanie i wydanie zamówienia oraz QR.

### Operacje

- [ ] Backup SQLite co najmniej dziennie + test odtworzenia.
- [ ] Monitoring procesu, portu, miejsca na dysku i błędów w journalu.
- [ ] Procedura aktualizacji z backupem i testem na kopii.
- [ ] Polityka retencji zamówień i audytu.
- [ ] Kontakt do odpowiedzialnego za system i procedura incydentu.

## 12. Zgłaszanie podatności

Jeśli znajdziesz podatność, nie publikuj jej publicznie przed
skoordynowaniem poprawki. Skontaktuj się z właścicielem repozytorium
(`nrosa2009r-lgtm`) i podaj:

- wersję/commit,
- opis i warunki reprodukcji,
- wpływ i proponowaną poprawkę (jeśli znasz),
- informację, czy dane są publiczne.

Nie dołączaj prawdziwych haseł, tokenów sesji ani danych uczniów.
