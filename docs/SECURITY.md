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
- Startowe hasło admina **nie jest wypisywane w konsoli ani w logach**;
  trafia wyłącznie do `admin_start.txt` z uprawnieniami `0600`. Plik leży
  w katalogu roboczym `/var/lib/zsl-order`, poza serwowanym `frontend/`, więc nie jest
  dostępny przez HTTP. Usuń go po pierwszym logowaniu i zmianie hasła
  (aplikacja robi to automatycznie; w razie błędu usuń ręcznie).
- Wymuszona zmiana hasła (`must_change_password`) blokuje całe API poza
  `me`, `logout`, `change-password`.
- Polityka nowego hasła:
  - uczeń: 10–128 znaków,
  - kuchnia i administrator: 12–128 znaków,
  - hasło nie może być identyczne z loginem ani zawierać loginu,
  - krótka lista najpopularniejszych haseł (np. `password123`, `qwerty123`)
    jest odrzucana,
  - bez wymogu „litera + cyfra” — liczy się długość i unikalność,
  - nowe hasło musi różnić się od obecnego,
  - hasła startowe generowane losowo nadal spełniają wyższe wymagania.
- Brak ujawniania, czy login istnieje: dla nieistniejącego konta wykonywane
  jest weryfikacja względem `dummy_hash`, aby wyrównać czas odpowiedzi.
- Rate limiting (w pamięci, konfigurowalny w `config.json`):
  - logowanie: blokada liczona dla pary **konto + IP** (`login_max_failures`,
    domyślnie 10 w 10 min → blokada 15 min),
  - logowanie: wysoki limit **per IP** (`login_ip_max_failures`, domyślnie
    300), bo sieć szkolna może współdzielić jeden publiczny adres,
  - kod odbioru: limit **per konto personelu** (`pickup_max_failures`,
    domyślnie 8) oraz wysoki limit per IP (`pickup_ip_max_failures`),
  - sukces czyści liczniki danej pary konto+IP oraz adresu IP.
- **Dlaczego konto + IP, a nie samo konto:** blokada per konto pozwalałaby
  obcej osobie z dowolnego miejsca zablokować administratora lub kuchnię,
  wysyłając kilka błędnych haseł (celowy „lockout”). Para konto + IP
  sprawia, że atakujący blokuje tylko siebie; prawdziwy użytkownik z innej
  sieci nadal się zaloguje. Residualne ryzyko: ktoś z tego samego publicznego
  adresu (np. inny uczeń w szkolnej Wi-Fi) może zablokować konto+IP na czas
  blokady. Dlatego blokada jest krótka (15 min) i konfigurowalna, a limit per
  IP (300) ogranicza masowe próby. Nie stosujemy globalnej blokady konta,
  bo bezpieczeństwo kont uprzywilejowanych jest ważniejsze niż wygoda.
- Limiter pamięci ma **górny limit 10 000 kluczy**; przy osiągnięciu progu
  usuwa wpisy wygasłe, a jeśli wszystkie są aktywne — najstarsze. Dzięki
  temu wiele unikalnych loginów/adresów nie zwiększa pamięci bez ograniczeń.
- Próby logowania na **nieistniejące konto** są liczone tak samo jak na
  istniejące (para konto+IP i IP), a odpowiedź jest identyczna
  (`401 Nieprawidłowy login lub hasło.`). Czas odpowiedzi wyrównuje
  weryfikacja względem `dummy_hash`.
- Odpowiedzi zawierające jednorazowe hasła (import CSV, generowanie kont,
  reset hasła) mają `Cache-Control: no-store` i `Pragma: no-cache`.
  Hasła nie trafiają do logów technicznych ani do audytu — audyt importu
  zapisuje tylko kto wykonał import i ile kont/grup utworzono.
- Unieważnianie sesji:
  - zmiana hasła usuwa wszystkie pozostałe sesje użytkownika,
  - reset hasła przez administratora usuwa wszystkie sesje konta,
  - dezaktywacja lub zmiana roli usuwa wszystkie sesje konta,
  - administrator może unieważnić wszystkie sesje wybranego konta
    (`DELETE /api/admin/users/{id}/sessions`).
- Import CSV (`1A;30`) tworzy konta uczniów bez danych osobowych;
  klasa/grupa trafia do loginu i kodu konta, a hasła są pokazywane
  wyłącznie raz w odpowiedzi.
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
  - `Secure` zależnie od `COOKIE_SECURE` (`auto`, `true`, `false`);
    w trybie `tunnel` zawsze `true`.
- Brak `Domain` — ciasteczko jest host-only.
- Sesja zawiera `csrf_token`, `created_at`, `expires_at`, `last_seen_at`.
- `last_seen_at` jest odświeżane maks. raz na 5 minut (mniej zapisów).
- Wylogowanie usuwa sesję. Zmiana hasła usuwa wszystkie pozostałe sesje.
  Dezaktywacja konta lub zmiana roli usuwa wszystkie sesje.
- Wygasłe sesje są czyszczone przy logowaniu i starcie.
- Brak „remember me” i brak sesji trwałych poza `session_hours`.

### Tokeny dostępowe JWT (API)

Obok sesji ciasteczkowej działa drugi, opcjonalny kanał: krótkotrwałe
tokeny **JWT HS256** dla klientów API.

- Wydawanie: `POST /api/auth/token` po weryfikacji hasła (ten sam limiter
  i te same zabezpieczenia czasowe co logowanie).
- TTL: **15 minut** (`JWT_TTL_SECONDS = 900`). Brak refresh tokenów;
  odświeżanie przez ponowne uwierzytelnienie.
- Claims: `sub`, `login`, `role`, `code`, `jti`, `iss`, `aud`, `iat`,
  `exp` — wyłącznie pseudonimowe identyfikatory, bez PII.
- Sekret: plik **`jwt.key`** tworzony przy pierwszym starcie, 64+ znaki
  hex, uprawnienia `0600` (wymuszane także przy każdym odczycie),
  wpisany do `.gitignore`. Usunięcie pliku unieważnia wszystkie tokeny
  (nowy sekret przy starcie).
- Rewokacja: każdy token ma wiersz w `sessions` z
  `token_hash = SHA-256("jwt:" + jti)`. Middleware wymaga zgodności
  podpisu, `exp`, `iss`, `aud`, istnienia wiersza, `active` oraz
  aktualnych `login`/`role`/`code` z bazy. Dzięki temu logout, zmiana
  hasła, zmiana roli, dezaktywacja i usunięcie konta działają
  **natychmiast**.
- `jti` ma 128 bitów losowości, więc sam wyciek sekretu nie pozwala
  zgadnąć istniejącego `jti`; mimo to sekret należy traktować jak
  klucz kryptograficzny.
- Token Bearer nie jest wysyłany automatycznie przez przeglądarkę,
  dlatego ścieżka Bearer nie wymaga CSRF. `origin_guard` nadal działa.
- Token przesyłany wyłącznie po HTTPS; nie umieszczać go w URL,
  logach, referrerze ani repozytorium. Po stronie klienta trzymać
  wyłącznie w pamięci lub bezpiecznym magazynie.

## 4. CSRF i kontrola pochodzenia

Dla metod `POST`/`PUT`/`PATCH`/`DELETE` na trasach chronionych w trybie
**ciasteczkowym**:

1. Wymagany nagłówek `X-CSRF-Token` zgodny z tokenem sesji.
2. Porównanie w czasie stałym (`constant_time_eq`).
3. Brak/niezgodność → `403 forbidden`.

Tryb **Bearer** nie wymaga CSRF: przeglądarka nie dołącza nagłówka
`Authorization` sama, a token nie jest ciasteczkiem. To celowe i
bezpieczne, dopóki klient nie przechowuje tokenu w miejscu dostępnym dla
XSS (frontend aplikacji używa ciasteczek i CSRF).

Dodatkowo `origin_guard` (wszystkie metody zmieniające stan, także login
i żądania Bearer):

- `Sec-Fetch-Site: cross-site` → `403`,
- w trybie `local`: jeśli jest `Origin`, jego host musi być równy
  bieżącemu `Host`, wskazywać `PUBLIC_URL` albo adres pętlowy,
- w trybie `tunnel`: `Host` musi wskazywać `PUBLIC_URL` (wyjątek:
  lokalne adresy pętlowe do diagnostyki), a `Origin` musi być równy
  `PUBLIC_URL`,
- `null`, pusty lub niezgodny `Origin` → `403`.

Tryb `local` ignoruje `CF-Connecting-IP` i inne nagłówki proxy. W trybie
`tunnel` adres klienta pochodzi z `CF-Connecting-IP` wyłącznie wtedy, gdy
połączenie przyszło z adresu z `TRUSTED_PROXIES`; w przeciwnym razie
używany jest adres gniazda TCP.

Jeśli w trybie `tunnel` zaufane proxy **nie przekazało** nagłówka
`CF-Connecting-IP` albo wartość jest nieprawidłowa (pusta, wiele adresów,
nie-IP), aplikacja:

- **nie udaje**, że wszystko jest w porządku — zapisuje ostrzeżenie
  `WARN` w logach (maks. raz na minutę, aby nie zalać logów),
- zachowuje adres gniazda TCP, czyli adres proxy (`127.0.0.1`).
  Wszystkie takie żądania trafiają wtedy do jednego koszyka limitów
  per IP; nie jest to ciche przypisanie adresu konkretnemu użytkownikowi
  i nie służy do identyfikacji osób,
- głównym mechanizmem limitowania pozostaje blokada pary konto+IP, a limit
  per IP jest wysoki i konfigurowalny — właśnie dlatego, że sieć szkolna
  może współdzielić jeden publiczny adres.

Ostrzeżenie w logach jest sygnałem, że należy sprawdzić konfigurację
`cloudflared` (nagłówek jest dodawany automatycznie) oraz zgodność
`TRUSTED_PROXIES` z adresem, z którego łączy się tunel. Bez tego
mechanizmu błędna konfiguracja po cichu sprowadzałaby wszystkich
użytkowników do `127.0.0.1`.

Żądania bez `Origin` są dozwolone (np. `curl`). W trybie ciasteczkowym
nadal obowiązuje CSRF; w trybie Bearer nie ma czego chronić przed
automatycznym dołączeniem. Nagłówek `Origin` nie jest jedynym
mechanizmem — to warstwa dodatkowa.

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

W trybie `tunnel` dodatkowo:

```http
Strict-Transport-Security: max-age=31536000
Content-Security-Policy: ...; upgrade-insecure-requests
```

HSTS nie zawiera `includeSubDomains` ani `preload` — aplikacja nie może
zakładać, że wszystkie subdomeny szkoły używają wyłącznie HTTPS. HSTS
i `upgrade-insecure-requests` nie są wysyłane w trybie `local`, aby nie
wymuszać HTTPS w sieci szkolnej. CSP bez inline, `frame-ancestors 'none'`,
`nosniff` i pozostałe nagłówki działają w obu trybach.

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
- Zawartość QR: wyłącznie token — bez loginu, kodu ucznia, ID konta,
  ceny, URL i tokenu sesji.
- Krótki kod: 6 liter z 24-znakowego alfabetu (bez cyfr i znaków
  mylonych), unikalny w bazie; ok. 28 bitów entropii.
- Krótki kod **nie jest publicznym sekretem** — jego użycie wymaga
  zalogowanego pracownika (`kitchen`/`admin`) i przechodzi przez CSRF.
- QR jest generowany tylko dla właściciela zamówienia; inny uczeń dostaje
  `404`.
- Anulowane, wydane i nieodebrane zamówienie nie generuje QR (`409`).
- Pickup wymaga `kitchen`/`admin`; uczeń dostaje `403`. Akceptuje tylko
  dzisiejsze zamówienie w statusie `ready` — po wydaniu kod jest
  jednorazowo zużyty. Pickup nie zmienia statusu; wydanie jest osobną
  akcją personelu.
- Token QR jest przechowywany jawnie w bazie (konieczny do generowania
  obrazu). Ochrona bazy = ochrona kodów odbioru.

## 9. Audyt i logowanie

- Tabela `audit_log`: `actor_id`, `action`, `details`, `created_at`.
- Zdarzenia m.in.: `login`, `token_issued`, `logout`, `password_changed`,
  `password_change_failed`, `password_reset`, `users_generated`,
  `users_exported`, `user_updated`, `user_deleted`, `order_created`,
  `order_cancelled_by_student`, `order_updated`, `pickup_lookup`,
  `menu_item_created`, `menu_item_updated`, `menu_item_hidden`,
  `settings_updated`, `admin_seeded`.
- `details` zawierają identyfikatory techniczne i statusy, nie imiona ani
  dane kontaktowe.
- Retencja: wpisy starsze niż `RETENTION_AUDIT_DAYS` (domyślnie 90 dni)
  są usuwane przez nocne zadanie retencji o `RETENTION_RUN_AT`
  (domyślnie 04:00), porcjami po 1000 rekordów.
- Logi techniczne: `tracing` pisze **JSON na stdout**, a systemd przekazuje
  je do journald. Aplikacja nie tworzy ani nie rotuje plików `logs/`.
- Middleware `request_log` zapisuje wyłącznie metodę, ścieżkę **bez
  query string**, status HTTP i czas odpowiedzi. **Nie** loguje
  nagłówków, ciasteczek, `Authorization`, `X-CSRF-Token`, ciał żądań,
  adresów IP ani tokenów JWT.
- Adres IP trafia do logów **wyłącznie** w zdarzeniach bezpieczeństwa:
  `login_failed`, `login_locked` i `pickup_locked`. Zamiast loginu
  zapisywane jest `account_id`, jeśli konto istnieje; nie logujemy
  istnienia konta ani wprowadzonego loginu.
- Logi mogą zawierać identyfikatory techniczne (np. `account_id`), role i
  komunikaty błędów; nie zawierają haseł ani sekretów. Journald ogranicz
  przez `SystemMaxUse` i `MaxRetentionSec` (np. 30 dni) — szczegóły w
  [DEPLOYMENT.md](DEPLOYMENT.md).
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
| Logi HTTP | JSON w journald bez wbudowanego limitu rozmiaru | `SystemMaxUse` i `MaxRetentionSec` w journald; IP tylko w zdarzeniach bezpieczeństwa; nie logować nagłówków i ciał |
| Tokeny JWT | 15 min, brak refresh tokenów; wymagają bezpiecznego przechowywania u klienta | HTTPS, pamięć/bezpieczny magazyn, nie logować; po podejrzeniu wycieku usunąć sesje JWT i `jwt.key` |
| Sekret `jwt.key` | leży na dysku obok bazy | `0600`, właściciel usługi, backup tylko szyfrowany; rotacja = usunięcie pliku i restart (unieważnia wszystkie tokeny) |
| Plik `admin_start.txt` | może pozostać, jeśli hasło nie zostanie zmienione | jak najszybciej zmienić hasło; plik ma `0600` |
| Wszyscy admini nieaktywni | bootstrap tworzy konto tylko przy zerze rekordów `admin` | nie dezaktywować ostatniego admina; procedura awaryjna przez DB |
| CSRF dla klientów API | w trybie ciasteczkowym wymaga nagłówka; klienci bez niego dostaną 403 | udokumentowane w [API.md](API.md); w trybie Bearer CSRF nie obowiązuje |

## 11. Checklista wdrożeniowa

### Konfiguracja

- [ ] `.env` na podstawie `.env.example`, uprawnienia `0600`.
- [ ] `DEPLOY_MODE` zgodny z wdrożeniem (`local` albo `tunnel`).
- [ ] W trybie `tunnel`: `BIND_ADDR=127.0.0.1:PORT`, poprawny
      `PUBLIC_URL` i `TRUSTED_PROXIES` ograniczone do lokalnego proxy.
- [ ] `COOKIE_SECURE` zgodne z transportem; w `tunnel` i tak wymuszone
      `true`.
- [ ] `session_hours` zgodne z polityką (domyślnie 12 h).
- [ ] `max_connections` rozsądne (8–16).
- [ ] `config.json`, `.env` i `baza.db` z uprawnieniami `0600`, właściciel
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
- [ ] Sprawdzone wydanie tokenu JWT (`POST /api/auth/token`) i żądanie
      Bearer bez CSRF; logout Bearer unieważnia token (401).
- [ ] `jwt.key` istnieje, ma `0600` i należy do użytkownika usługi.
- [ ] Journald ma ustawione `SystemMaxUse` i `MaxRetentionSec` (np. 30
      dni); `journalctl -u zsl-order -o cat | jq .` zwraca poprawny JSON.

### Operacje

- [ ] Backup SQLite co najmniej dziennie + test odtworzenia.
- [ ] Monitoring procesu, portu, miejsca na dysku i błędów w journalu.
- [ ] Procedura aktualizacji z backupem i testem na kopii.
- [ ] Polityka retencji zamówień, kluczy idempotencji i audytu;
      `RETENTION_ORDERS_DAYS=0` oznacza brak retencji zamówień i generuje
      ostrzeżenie przy starcie — ustaw świadomie i sprawdź, czy raporty z
      usuniętych dni mogą być puste.
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
