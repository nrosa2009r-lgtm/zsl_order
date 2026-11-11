# API — ZSL Order

Pełny opis API HTTP aplikacji ZSL Order. Wszystkie ścieżki są względne
wobec adresu serwera, np. `http://127.0.0.1:3000`.

- Format: JSON UTF-8 (`Content-Type: application/json`).
- Odpowiedzi błędów: JSON `{ "error": "...", "code": "..." }`.
- Czas: znaczniki Unix w sekundach (UTC). `pickup_time` to lokalna godzina
  w formacie `HH:MM`.
- Kwoty: liczby całkowite w groszach (`total_cents`, `price_cents`).
- API nie korzysta z wersjonowania w ścieżce; kontrakt opisuje bieżąca
  wersja aplikacji (`1.0.0`).

## Spis treści

- [Uwierzytelnianie i CSRF](#uwierzytelnianie-i-csrf)
- [Role i macierz endpointów](#role-i-macierz-endpointów)
- [Publiczne](#publiczne)
- [Sesja i hasło](#sesja-i-hasło)
- [Uczeń](#uczeń)
- [Kuchnia i administrator](#kuchnia-i-administrator)
- [Administrator](#administrator)
- [Modele danych](#modele-danych)
- [Błędy](#błędy)
- [Walidacja](#walidacja)
- [Limity i rate limiting](#limity-i-rate-limiting)
- [Przykład sesji curl](#przykład-sesji-curl)

---

## Uwierzytelnianie i CSRF

### Ciasteczko sesji

Po `POST /api/auth/login` serwer ustawia ciasteczko:

```http
Set-Cookie: zsl_session=<64 znaki hex>; Path=/; HttpOnly; SameSite=Strict; Max-Age=43200
```

- 32 losowe bajty → 64 znaki hex.
- W bazie przechowywany jest wyłącznie SHA-256 tokenu.
- `Secure` jest dodawane, gdy `secure_cookies = true` w `config.json`.
- `SameSite=Strict` oznacza, że ciasteczko nie jest wysyłane przy żądaniach
  cross-site.
- Czas życia: `session_hours` (domyślnie 12 h).

### Token dostępowy JWT (`Authorization: Bearer`)

Oprócz sesji ciasteczkowej aplikacja wydaje krótkotrwałe tokeny **JWT**
dla klientów API (skrypty, integracje, aplikacje mobilne). Token uzyskuje
się przez `POST /api/auth/token` z tym samym loginem i hasłem co przy
logowaniu.

```http
Authorization: Bearer <access_token>
```

- Algorytm: **HS256**, sekret HMAC w pliku `jwt.key` (0600, generowany
  przy pierwszym starcie, poza repozytorium).
- Czas życia: **15 minut** (`expires_in: 900`). Odświeżanie odbywa się
  przez ponowne wywołanie `POST /api/auth/token`.
- Claims: `sub` (ID konta), `login`, `role`, `code`, `jti`, `iss`
  (`zsl-order`), `aud` (`zsl-order-api`), `iat`, `exp`. Brak danych
  osobowych.
- Każdy token ma odpowiadający rekord w tabeli `sessions`
  (`token_hash = SHA-256("jwt:" + jti)`), więc wylogowanie, zmiana hasła,
  zmiana roli, dezaktywacja konta i twarde usunięcie konta unieważniają
  token **natychmiast**, bez czekania na `exp`.
- `POST /api/auth/logout` z nagłówkiem Bearer usuwa wyłącznie tę sesję
  tokenu; nie usuwa sesji ciasteczkowej przeglądarki.
- Dla żądań Bearer **nie trzeba** wysyłać `X-CSRF-Token` — przeglądarka
  nie dołącza nagłówka `Authorization` automatycznie, więc klasyczny CSRF
  nie ma zastosowania. `origin_guard` nadal obowiązuje dla metod
  zmieniających stan (np. obcy `Origin` → `403`).
- Token należy przesyłać wyłącznie po HTTPS i trzymać poza logami,
  URL-ami i repozytoriami. Nie ma mechanizmu refresh tokenów.

### Token CSRF

Odpowiedź logowania oraz `GET /api/auth/me` zawierają `csrf_token`
(64 znaki hex). Token jest przypisany do sesji.

Dla każdej metody zmieniającej stan (`POST`, `PUT`, `PATCH`, `DELETE`)
w trybie **ciasteczkowym** należy dodać nagłówek:

```http
X-CSRF-Token: <csrf_token>
```

Brak lub niezgodność → `403 forbidden`. Token jest porównywany w czasie
stałym. W trybie Bearer CSRF nie jest wymagany (patrz wyżej). Dodatkowo
działa `origin_guard` dla obu trybów:

- `Sec-Fetch-Site: cross-site` → `403`,
- jeśli nagłówek `Origin` jest obecny, jego host musi być zgodny z `Host`
  (`null`, pusty lub inny host → `403`).

Żądania bez `Origin` (np. `curl`, klient API) są dozwolone, ale w trybie
ciasteczkowym nadal wymagają CSRF dla metod zmieniających stan. Klient
Bearer nie wysyła CSRF.

### Wymuszona zmiana hasła

Konto z `must_change_password = true` może wywołać wyłącznie:

- `GET /api/auth/me`,
- `POST /api/auth/logout`,
- `POST /api/auth/change-password`.

Każdy inny uwierzytelniony endpoint zwraca:

```http
HTTP/1.1 403 Forbidden
{"error":"Najpierw zmień hasło startowe.","code":"password_change_required"}
```

### Wylogowanie i unieważnianie sesji

- `POST /api/auth/logout` usuwa bieżącą sesję (ciasteczkową albo Bearer).
- Zmiana hasła usuwa wszystkie pozostałe sesje użytkownika, w tym sesje
  powiązane z tokenami JWT.
- Dezaktywacja konta, zmiana roli lub twarde usunięcie konta usuwa
  wszystkie sesje konta i unieważnia wydane tokeny JWT.
- Wygasłe sesje (także wpisy JWT) są usuwane przy logowaniu, wydaniu
  tokenu i starcie aplikacji.
- Usunięcie pliku `jwt.key` unieważnia **wszystkie** tokeny JWT, ale nie
  wpływa na sesje ciasteczkowe.

## Role i macierz endpointów

| Metoda | Ścieżka | Wymagana rola | CSRF |
| --- | --- | --- | --- |
| `GET` | `/api/public/info` | — | nie |
| `GET` | `/api/healthz` | — | nie |
| `POST` | `/api/auth/login` | — | nie (chroni origin guard) |
| `POST` | `/api/auth/token` | — | nie (Bearer; chroni origin guard) |
| `GET` | `/api/auth/me` | zalogowany | nie |
| `POST` | `/api/auth/logout` | zalogowany | tak |
| `POST` | `/api/auth/change-password` | zalogowany | tak |
| `GET` | `/api/student/menu` | `student` | nie |
| `GET` | `/api/student/orders` | `student` | nie |
| `POST` | `/api/student/orders` | `student` | tak |
| `POST` | `/api/student/orders/:id/cancel` | `student` (właściciel) | tak |
| `GET` | `/api/student/orders/:id/qr.svg` | `student` (właściciel) | nie |
| `GET` | `/api/staff/menu` | `kitchen` / `admin` | nie |
| `POST` | `/api/staff/menu` | `kitchen` / `admin` | tak |
| `PUT` | `/api/staff/menu/:id` | `kitchen` / `admin` | tak |
| `DELETE` | `/api/staff/menu/:id` | `kitchen` / `admin` | tak |
| `GET` | `/api/staff/orders` | `kitchen` / `admin` | nie |
| `PATCH` | `/api/staff/orders/:id` | `kitchen` / `admin` | tak |
| `POST` | `/api/staff/pickup` | `kitchen` / `admin` | tak |
| `GET` | `/api/staff/report?date=RRRR-MM-DD` | `kitchen` / `admin` | nie |
| `GET` | `/api/staff/report.csv?date=RRRR-MM-DD` | `kitchen` / `admin` | nie |
| `GET` | `/api/admin/stats` | `admin` | nie |
| `GET` | `/api/admin/stats/activity` | `admin` | nie |
| `GET` | `/api/admin/users` | `admin` | nie |
| `GET` | `/api/admin/users/export.csv` | `admin` | nie |
| `POST` | `/api/admin/users/generate` | `admin` | tak |
| `POST` | `/api/admin/users/import` | `admin` | tak |
| `PATCH` | `/api/admin/users/:id` | `admin` | tak |
| `DELETE` | `/api/admin/users/:id` | `admin` | tak |
| `POST` | `/api/admin/users/:id/reset-password` | `admin` | tak |
| `DELETE` | `/api/admin/users/:id/sessions` | `admin` | tak |
| `GET` | `/api/admin/settings` | `admin` | nie |
| `PUT` | `/api/admin/settings` | `admin` | tak |
| `GET` | `/api/admin/audit` | `admin` | nie |

Uwaga: kolumna **CSRF** dotyczy trybu ciasteczkowego. Żądania z
`Authorization: Bearer` nie wymagają `X-CSRF-Token`.

Uwaga: endpointy `/api/staff/*` są wspólne dla kuchni i administratora.
Administrator używa ich do zarządzania menu i zamówieniami; kuchnia ma
dodatkowe ograniczenia opisane w sekcji zamówień.

## Publiczne

### `GET /api/public/info`

Bez uwierzytelnienia. Informacje potrzebne na stronie logowania.

**Odpowiedź 200**

```json
{
  "cafeteria_name": "Stołówka ZSŁ",
  "ordering_enabled": true,
  "payment_info": "Płatność przy odbiorze: gotówka lub karta."
}
```

### `GET /api/healthz`

Publiczny test żywotności dla monitoringu. Sprawdza, czy aplikacja i baza
SQLite odpowiadają. Nie wymaga logowania i nie zwraca wersji ani
konfiguracji.

**Odpowiedź 200**

```json
{"status": "ok"}
```

### `POST /api/auth/login`

```json
{
  "login": "s-a7k2m9p4qx",
  "password": "TwojeHaslo123"
}
```

**Odpowiedź 200**

```json
{
  "user": {
    "login": "s-a7k2m9p4qx",
    "role": "student",
    "display_code": "UCZ-AB12C",
    "must_change_password": false
  },
  "csrf_token": "<64 znaki hex>"
}
```

Nagłówek `Set-Cookie` ustawia sesję.

**Błędy**

| Status | `code` | Opis |
| --- | --- | --- |
| 401 | `unauthorized` | pusty login/hasło, przekroczone długości (login > 64, hasło > 256) lub nieprawidłowe dane |
| 429 | `too_many_requests` | rate limit; nagłówek `Retry-After` w sekundach |

Limity wejścia: login ≤ 64 znaki, hasło ≤ 256 znaków. Login jest
przycinany z białych znaków.

### `POST /api/auth/token`

Wydaje krótkotrwały token JWT do uwierzytelniania API przez
`Authorization: Bearer`. Nie ustawia ciasteczka. Wymagane są te same
dane logowania i te same limity co przy `POST /api/auth/login`
(wspólny limiter prób).

```json
{
  "login": "s-a7k2m9p4qx",
  "password": "TwojeHaslo123"
}
```

**Odpowiedź 200**

```json
{
  "access_token": "<JWT HS256>",
  "token_type": "Bearer",
  "expires_in": 900,
  "user": {
    "login": "s-a7k2m9p4qx",
    "role": "student",
    "display_code": "UCZ-AB12C",
    "must_change_password": false
  }
}
```

- `expires_in` to liczba sekund (15 minut).
- Token jest podpisany HS256 i zawiera `iss=zsl-order`,
  `aud=zsl-order-api`, `jti`, `iat`, `exp`; nie zawiera danych osobowych.
- Wydanie tokenu tworzy wpis w `sessions`; jego usunięcie (logout,
  zmiana hasła, dezaktywacja konta) natychmiast unieważnia token.
- Konto z `must_change_password = true` otrzyma token, ale będzie mogło
  wywołać wyłącznie `me`, `logout` i `change-password` — tak samo jak
  w trybie ciasteczkowym.
- Każde wydanie tokenu jest audytowane jako `token_issued`.

**Błędy**

| Status | `code` | Opis |
| --- | --- | --- |
| 401 | `unauthorized` | nieprawidłowy login/hasło lub konto nieaktywne |
| 429 | `too_many_requests` | rate limit; nagłówek `Retry-After` w sekundach |

## Sesja i hasło

### `GET /api/auth/me`

Zwraca bieżącego użytkownika i token CSRF (przydatne po odświeżeniu strony).

```json
{
  "user": {
    "login": "k-9m2x7q4p1a",
    "role": "kitchen",
    "display_code": "KUC-XY7Z9",
    "must_change_password": false
  },
  "csrf_token": "<64 znaki hex>"
}
```

### `POST /api/auth/logout`

Puste ciało `{}` lub brak ciała. Zwraca `{"ok": true}`.

- W trybie ciasteczkowym czyści ciasteczko i usuwa sesję.
- W trybie Bearer usuwa rekord sesji powiązany z tokenem JWT; token
  natychmiast przestaje działać. Nagłówek `Authorization` jest wymagany,
  CSRF nie.

### `POST /api/auth/change-password`

```json
{
  "current_password": "StareHaslo123",
  "new_password": "NoweHaslo456"
}
```

Zasady nowego hasła:

- 10–128 znaków,
- co najmniej jedna litera ASCII i jedna cyfra,
- różne od obecnego.

**Odpowiedź 200**

```json
{"ok": true}
```

**Błędy**

| Status | `code` | Opis |
| --- | --- | --- |
| 400 | `bad_request` | nieprawidłowa długość/skład hasła lub hasło takie samo jak obecne |
| 400 | `bad_request` | błędne obecne hasło |
| 401 | `unauthorized` | sesja wygasła |

Po zmianie hasła przez administratora usuwany jest plik `admin_start.txt`
(jeśli istnieje).

## Uczeń

### `GET /api/student/menu`

Zwraca wyłącznie dostępne pozycje menu, posortowane po `sort_order`, nazwie.

```json
{
  "items": [
    {
      "id": 2,
      "name": "Kotlet schabowy z ziemniakami",
      "description": "Panierowany kotlet, ziemniaki, surówka",
      "price_cents": 1400,
      "category": "Dania główne",
      "allergens": "gluten, jaja, mleko",
      "available": true,
      "sort_order": 20
    }
  ],
  "ordering_enabled": true,
  "payment_info": "Płatność przy odbiorze: gotówka lub karta.",
  "max_order_items": 20,
  "cancel_window_minutes": 30,
  "pickup_limit_minutes": 120,
  "max_uncollected": 0
}
```

### `GET /api/student/orders`

Zwraca do 200 własnych zamówień, najnowsze pierwsze. Każde zamówienie ma
pozycje w kolejności `order_items.id`.

### `POST /api/student/orders`

```json
{
  "items": [
    {"menu_item_id": 2, "quantity": 2},
    {"menu_item_id": 6, "quantity": 1}
  ],
  "payment_method": "cash",
  "pickup_time": "12:30",
  "idempotency_key": "550e8400-e29b-41d4-a716-446655440000"
}
```

- `idempotency_key` (**wymagane**): 8–128 znaków `[A-Za-z0-9._:-]`.
  Klient generuje nowy klucz na jedno kliknięcie „Złóż zamówienie” i używa
  go ponownie wyłącznie przy ponowieniu tego samego żądania. Powtórzenie
  klucza zwraca **istniejące** zamówienie (`200`) zamiast tworzyć drugie.
  Klucz można też przesłać w nagłówku `Idempotency-Key`; jeśli występuje
  w obu miejscach, wartości muszą być identyczne. Indeks unikalny obejmuje
  parę `(user_id, idempotency_key)`, więc ten sam klucz użyty przez innego
  ucznia tworzy jego własne zamówienie i nigdy nie ujawnia cudzego.
- `items`: 1–`max_order_items` pozycji (domyślnie 20); `menu_item_id` musi
  istnieć i być dostępne; `quantity` 1–10; ta sama pozycja nie może wystąpić
  dwa razy. Pola `price_cents`, `total_cents`, `status` i
  `payment_status` w żądaniu są ignorowane — ceny i statusy ustala serwer.
- `payment_method`: `cash` lub `card`.
- `pickup_time`: `""` (brak) lub `HH:MM` w zakresie `00:00`–`23:59`.
- Suma jest liczona po stronie serwera na podstawie aktualnych cen.
- Maksymalna kwota zamówienia: 200 000 gr (2000 zł).
- Limit otwartych zamówień użytkownika: `max_open_orders` (domyślnie 3);
  statusy liczone jako otwarte: `new`, `preparing`, `ready`.
- `max_uncollected > 0` blokuje nowe zamówienia, jeśli uczeń ma co najmniej
  tyle nieodebranych zamówień w ostatnich 30 dniach. Blokadę można zdjąć
  akcją administratora `POST /api/admin/users/:id/clear-uncollected-block`;
  historia nieodebranych i raporty pozostają bez zmian.
- `ordering_enabled = false` blokuje składanie zamówień.

**Odpowiedź 201** — nowy `OrderView`. **Odpowiedź 200** — istniejące
zamówienie dla powtórzonego klucza idempotencji.

**Błędy**

| Status | `code` | Opis |
| --- | --- | --- |
| 400 | `bad_request` | nieprawidłowe pozycje, płatność, godzina, klucz idempotencji, kwota |
| 409 | `conflict` | zamawianie wyłączone, limit otwartych, blokada nieodebranych, pozycja niedostępna |

### `POST /api/student/orders/:id/cancel`

Puste ciało `{}`. Anulować można wyłącznie **własne**, nieopłacone
zamówienie w statusie `new`, w terminie `cancel_window_minutes` od
złożenia (`0` = bez limitu). Operacja jest transakcyjna i warunkowa —
przy dwóch równoczesnych anulowaniach jedno kończy się `200`, drugie
`409`. Zwraca zaktualizowany `OrderView` ze statusem `cancelled`.

| Status | `code` | Opis |
| --- | --- | --- |
| 404 | `not_found` | zamówienie nie istnieje lub należy do innego użytkownika |
| 409 | `conflict` | zamówienie przyjęte, opłacone, nieodebrane lub minął termin anulowania |

### `GET /api/student/orders/:id/qr.svg`

Zwraca SVG z kodem QR dla **własnego**, nieanulowanego zamówienia.

```http
Content-Type: image/svg+xml; charset=utf-8
Cache-Control: no-store
```

Zawartość QR: `ZSL:<48 znaków hex>` (token 24 bajtów). QR nie zawiera
loginu, kodu ucznia ani żadnych danych osobowych. Zamówienie innego
użytkownika lub anulowane → `404` / `409`.

## Kuchnia i administrator

### `GET /api/staff/menu`

Zwraca wszystkie pozycje, także ukryte (`available = false`), sortowane
najpierw dostępne, potem `sort_order`, nazwa.

### `POST /api/staff/menu`

```json
{
  "name": "Zupa pomidorowa",
  "description": "Klasyczna zupa z makaronem",
  "price_cents": 550,
  "category": "Zupy",
  "allergens": "gluten, seler",
  "available": true,
  "sort_order": 10
}
```

- `name`: 1–80 znaków (wymagane),
- `description`: ≤ 240 znaków (domyślnie `""`),
- `price_cents`: 0–100 000 (0–1000 zł),
- `category`: 1–40 znaków (domyślnie `"Dania"`),
- `allergens`: ≤ 160 znaków,
- `available`: bool (domyślnie `true`),
- `sort_order`: −10 000…10 000.

**Odpowiedź 201** — `MenuItemView`.

### `PUT /api/staff/menu/:id`

Przyjmuje ten sam obiekt co `POST` (pełna aktualizacja). Zwraca
`MenuItemView`. Brak pozycji → `404`.

### `DELETE /api/staff/menu/:id`

Miękkie ukrycie: ustawia `available = false`. Zwraca `{"ok": true}`.
Pozycja pozostaje widoczna dla personelu i w historycznych zamówieniach
(pozycje zamówień są snapshotami). Brak pozycji → `404`.

### `GET /api/staff/orders?scope=open|all|today`

- `open` (domyślnie): statusy `new`, `preparing`, `ready` bez znacznika
  nieodebranego, sortowanie najstarsze pierwsze (kolejka),
- `today`: zamówienia od początku doby `Europe/Warsaw`, najnowsze pierwsze,
- `all`: wszystkie, najnowsze pierwsze, maks. 300.

Każdy wariant zwraca maks. 300 zamówień. Odpowiedź to tablica
`OrderView` z pozycjami.

### `PATCH /api/staff/orders/:id`

```json
{"status": "ready"}
```

lub

```json
{"payment_status": "paid"}
```

lub oba pola. Dozwolone wartości:

- `status`: `new`, `preparing`, `ready`, `completed`, `cancelled`,
- `payment_status`: `unpaid`, `paid`.

Ograniczenia dla roli `kitchen`:

- nie może zmieniać zamówień z innych dni niż dzisiejszy (`403`),
- nie może zmieniać zamówień `completed`, `cancelled` ani nieodebranych
  (`409`),
- prowadzi zamówienie krok po kroku: `new → preparing → ready →
  completed` (`403` przy pominięciu kroku),
- może ustawić płatność tylko na `paid` i tylko gdy zamówienie jest
  `ready` lub `completed` (`403`/`409` dla innych przypadków),
- nie może oznaczyć jako opłacone zamówienia anulowanego ani nieodebranego,
- nie może cofnąć płatności (`paid → unpaid`); robi to wyłącznie
  administrator, a zdarzenie trafia do audytu jako
  `order_payment_reverted`,
- `completed` automatycznie ustawia `paid`, jeśli nie podano płatności,
- przy przejściu na `paid` serwer zapisuje `paid_at` i `paid_by`.

Administrator nie ma ograniczeń dnia ani przejść (poza walidacją wartości).
Zmiana statusu nieodebranego zamówienia przez administratora zdejmuje
znacznik `uncollected_at` i przywraca je do obiegu. Wszystkie zmiany są
transakcyjne; równoległa zmiana tego samego zamówienia kończy się `409`
zamiast nadpisania danych. Zwraca zaktualizowany `OrderView`. Brak
zamówienia → `404`.

### `POST /api/staff/pickup`

Wyszukuje zamówienie po krótkim kodzie lub tokenie QR. Nie zmienia statusu.

```json
{"code": "K7M2QP"}
```

Akceptowane postacie:

- `K7M2QP` — krótki kod (wielkość liter bez znaczenia),
- `ZSL:<48 znaków hex>` — pełna zawartość QR,
- `<48 znaków hex>` — sam token QR.

**Odpowiedź 200**

```json
{"order": { /* OrderView */ }}
```

| Status | `code` | Opis |
| --- | --- | --- |
| 400 | `bad_request` | pusty kod lub > 128 znaków |
| 404 | `not_found` | brak zamówienia o takim kodzie/tokenie |
| 409 | `conflict` | zamówienie anulowane lub nieodebrane |

Każde wyszukanie jest zapisywane w audycie (`pickup_lookup`).

### `GET /api/staff/report?date=RRRR-MM-DD`

Raport dzienny dla kuchni i administratora. `date` opcjonalne
(domyślnie dzisiaj). Doba jest liczona w **Europe/Warsaw** — także przy
zmianie czasu (dzień wiosenny ma 23 h, jesienny 25 h). Zakres obejmuje
zamówienia **złożone** w danym dniu. Odpowiedź zawiera sumy łączne oraz
podział na gotówkę i kartę:

```json
{
  "date": "2026-11-10",
  "orders": 12,
  "paid_orders": 9,
  "paid_cents": 18600,
  "unpaid_orders": 1,
  "unpaid_cents": 1400,
  "cancelled_orders": 1,
  "uncollected_orders": 1,
  "completed_orders": 9,
  "open_orders": 1,
  "cash": { "orders": 7, "paid_orders": 5, "paid_cents": 9800, "...": "..." },
  "card": { "orders": 5, "paid_orders": 4, "paid_cents": 8800, "...": "..." }
}
```

- `paid_orders`, `unpaid_orders`, `cancelled_orders` i `uncollected_orders`
  to **rozłączne** kategorie liczbowe:
  `paid_orders + unpaid_orders + cancelled_orders + uncollected_orders =
  orders`. Nieodebrane nie jest liczone jednocześnie jako opłacone,
  nieopłacone ani anulowane.
- `paid_cents` to utarg z opłaconych zamówień z wyjątkiem jawnie
  anulowanych; **opłacone nieodebrane nadal jest przychodem** i wchodzi do
  tej sumy (mimo że nie zwiększa `paid_orders`). Analogicznie
  `unpaid_cents` obejmuje nieopłacone nieodebrane.
- `cancelled_orders` nie zawiera nieodebranych (te mają osobny licznik),
- `completed_orders + open_orders + cancelled_orders + uncollected_orders =
  orders` (podział według realizacji),
- `cash.*` + `card.*` = wartości łączne,
- nieprawidłowa data → `400`.

### `GET /api/staff/report.csv?date=RRRR-MM-DD`

Ten sam raport jako CSV (`text/csv; charset=utf-8`, BOM, separator `;`,
`Cache-Control: no-store`). Wiersze: gotówka, karta, razem. Kolumny:
data, metoda, zamówienia, opłacone, kwota opłacona w groszach,
nieopłacone, kwota nieopłacona w groszach, anulowane, nieodebrane.

## Administrator

### `GET /api/admin/stats`

```json
{
  "total_users": 120,
  "students": 100,
  "kitchen": 4,
  "admins": 2,
  "orders_today": 37,
  "revenue_today_cents": 48200,
  "open_orders": 5,
  "awaiting_payment": 3,
  "menu_available": 7,
  "active_sessions": 8
}
```

- `students`, `kitchen`, `admins` — tylko aktywne konta,
- `orders_today`, `revenue_today_cents` — od początku doby `Europe/Warsaw`;
  utarg liczy tylko `payment_status = 'paid'` i status inny niż `cancelled`,
- `open_orders` — `new`, `preparing`, `ready`,
- `awaiting_payment` — `unpaid` i status inny niż `cancelled`,
- `active_sessions` — sesje niewygasłe.

### `GET /api/admin/stats/activity?days=7|14|30|90`

Statystyki aktywności dla wykresów. `days` jest przycinane do zakresu
1–90 (domyślnie 14). Dni i godziny są liczone w **Europe/Warsaw**
(niezależnie od strefy serwera); znaczniki czasu pozostają w UTC.

**Odpowiedź 200**

```json
{
  "days": 14,
  "from": "2026-10-25",
  "to": "2026-11-07",
  "daily": [
    {
      "date": "2026-10-25",
      "orders": 42,
      "revenue_cents": 51200,
      "completed": 38,
      "cancelled": 2
    }
  ],
  "hourly_today": [
    {"hour": 0, "orders": 0}
  ],
  "status_counts": {
    "new": 3,
    "preparing": 2,
    "ready": 1,
    "completed": 38,
    "cancelled": 2
  },
  "payment_counts": {"unpaid": 4, "paid": 42},
  "top_items": [
    {"name": "Kotlet schabowy z ziemniakami", "quantity": 21, "revenue_cents": 29400}
  ],
  "totals": {
    "orders": 46,
    "completed": 38,
    "cancelled": 2,
    "revenue_cents": 51200,
    "non_cancelled_orders": 44,
    "average_order_cents": 1163
  }
}
```

- `daily` zawsze zawiera dokładnie `days` punktów (także dni bez zamówień),
  od `from` do `to` włącznie.
- `hourly_today` zawsze zawiera 24 punkty (godziny 0–23).
- `revenue_cents` liczy tylko zamówienia `payment_status = 'paid'` i status
  inny niż `cancelled`.
- `top_items` to maks. 8 pozycji w zakresie, bez anulowanych zamówień.
- Odpowiedź nie zawiera danych osobowych.

### `GET /api/admin/users`

Parametry zapytania (wszystkie opcjonalne):

| Parametr | Wartość |
| --- | --- |
| `role` | `student`, `kitchen`, `admin` |
| `active` | `true` / `false` |
| `q` | fragment loginu lub `display_code` (max 40 znaków) |

Maks. 500 wyników, najnowsze konta pierwsze.

```json
[
  {
    "id": 12,
    "login": "s-a7k2m9p4qx",
    "role": "student",
    "display_code": "UCZ-AB12C",
    "active": true,
    "must_change_password": false,
    "created_at": 1762450000,
    "last_login_at": 1762451234
  }
]
```

### `GET /api/admin/users/export.csv`

Eksport **metadanych** kont do pliku CSV. Akceptuje te same parametry co
`GET /api/admin/users` (`role`, `active`, `q`). Maks. 20 000 wierszy,
najnowsze konta pierwsze.

- `Content-Type: text/csv; charset=utf-8`
- `Content-Disposition: attachment; filename="konta-zsl-order-RRRR-MM-DD.csv"`
- `Cache-Control: no-store`
- BOM UTF-8 i CRLF (wygodne w Excelu), separator `;`.
- Każda komórka jest w cudzysłowach; `"` jest escapowany przez `""`.
- Wartości zaczynające się od `=`, `+`, `-`, `@` są poprzedzane `'`
  (ochrona przed formułami arkusza kalkulacyjnego).

Nagłówek:

```text
login;rola;kod;aktywne;haslo_startowe;utworzono;ostatnie_logowanie
```

| Kolumna | Znaczenie |
| --- | --- |
| `login` | login konta |
| `rola` | `student` / `kitchen` / `admin` |
| `kod` | pseudonimowy `display_code` |
| `aktywne` | `tak` / `nie` |
| `haslo_startowe` | `tak`, jeśli konto nadal wymaga zmiany hasła — **to nie jest hasło** |
| `utworzono` | data i godzina `RRRR-MM-DD HH:MM` w Europe/Warsaw |
| `ostatnie_logowanie` | data i godzina w Europe/Warsaw lub puste |

**Eksport nie zawiera haseł i nie może ich zawierać** — baza przechowuje
wyłącznie skróty Argon2id. Hasła są dostępne tylko w odpowiedzi
`POST /api/admin/users/generate` oraz `POST /api/admin/users/:id/reset-password`.
Eksport jest zapisywany w audycie jako `users_exported`.

### `POST /api/admin/users/generate`

```json
{"role": "student", "count": 30}
```

- `role`: `student`, `kitchen` lub `admin`,
- `count`: 1–100,
- łączna liczba kont w systemie nie może przekroczyć 20 000.

**Odpowiedź 200**

```json
{
  "accounts": [
    {
      "login": "s-a7k2m9p4qx",
      "password": "Xk7...",
      "role": "student",
      "display_code": "UCZ-AB12C"
    }
  ],
  "note": "Hasła są widoczne tylko teraz. Zapisz je i przekaż uczniom w bezpieczny sposób."
}
```

**Hasła są zwracane wyłącznie w tej odpowiedzi** — nie są przechowywane
w postaci jawnej. Nowe konta mają `must_change_password = true`.
Odpowiedzi z hasłami mają `Cache-Control: no-store` i `Pragma: no-cache`.

### `POST /api/admin/users/import`

Import uczniów z CSV. Treść żądania to czysty tekst CSV
(`Content-Type: text/csv`), jedna grupa w wierszu:

```text
klasa;liczba
1A;30
2B;25
```

- format `klasa;liczba` (akceptowany też przecinek), np. `1A;30`,
- bez imion i nazwisk,
- nazwa klasy/grupy: 1–16 znaków (litery ASCII, cyfry, `-`, `_`),
- 1–100 kont na grupę, maks. 200 na jedno żądanie,
- wiersze zaczynające się od `#` są komentarzami, nagłówek jest pomijany.

Klasa trafia do loginu (`s-1a-...`) i kodu konta (`UCZ-1A-...`), więc można
ją odfiltrować w wyszukiwaniu kont. Odpowiedź zawiera konta z hasłami
**tylko ten jeden raz** (jak przy generowaniu) oraz podsumowanie grup.
Audyt zapisuje `users_imported`.

**Odpowiedź 200**

```json
{
  "accounts": [
    {
      "login": "s-1a-a7k2m9p4qx",
      "password": "Xk7...",
      "role": "student",
      "display_code": "UCZ-1A-AB12C",
      "group": "1A"
    }
  ],
  "groups": [{"group": "1A", "count": 30}],
  "note": "Hasła są widoczne tylko teraz. Wydrukuj lub zapisz je i przekaż uczniom w bezpieczny sposób."
}
```

Odpowiedź ma `Cache-Control: no-store` i `Pragma: no-cache`; hasła nie
trafiają do logów ani audytu. Audyt zapisuje `users_imported` z liczbą
kont i grup.

### `PATCH /api/admin/users/:id`

```json
{"active": false}
```

lub

```json
{"role": "kitchen"}
```

lub oba. Zmiana roli musi być jedną z: `student`, `kitchen`, `admin`.
Dezaktywacja lub zmiana roli unieważnia wszystkie sesje konta.
Administrator nie może dezaktywować ani zmienić roli własnego konta
(`400 bad_request`). Brak konta → `404`.

### `DELETE /api/admin/users/:id`

Trwale usuwa konto wraz z:

- wszystkimi zamówieniami tego konta,
- pozycjami tych zamówień,
- wszystkimi sesjami konta.

Operacja jest **nieodwracalna**. Zabezpieczenia:

- nie można usunąć własnego konta (`400`),
- nie można usunąć ostatniego aktywnego administratora (`409`),
- brak konta → `404`.

Wszystko wykonuje się w jednej transakcji; przy błędzie nie pozostają
osierocone wiersze. Zdarzenie trafia do audytu jako `user_deleted`.

**Odpowiedź 200**

```json
{
  "ok": true,
  "login": "s-a7k2m9p4qx",
  "deleted_orders": 4
}
```

### `POST /api/admin/users/:id/reset-password`

Puste ciało `{}`. Zwraca nowe hasło **tylko raz**:

```json
{
  "login": "s-a7k2m9p4qx",
  "password": "NoweLosoweHaslo1",
  "note": "Nowe hasło jest widoczne tylko teraz."
}
```

Reset ustawia `must_change_password = true` i usuwa wszystkie sesje konta.
Nie można resetować własnego hasła (`400`); własne hasło zmienia się przez
`/api/auth/change-password`. Brak konta → `404`.

### `DELETE /api/admin/users/:id/sessions`

Unieważnia wszystkie sesje wybranego konta (ciasteczka i tokeny JWT).
Przydatne po zgubieniu urządzenia lub podejrzeniu kradzieży sesji. Nie
można użyć wobec własnego konta (`400`) — do tego służy
`POST /api/auth/logout`. Brak konta → `404`. Audyt zapisuje
`sessions_revoked`.

**Odpowiedź 200**

```json
{"ok": true, "revoked": 2}
```

### `POST /api/admin/users/:id/clear-uncollected-block`

Puste ciało `{}`. Zdejmuje blokadę nowych zamówień wynikającą z limitu
`max_uncollected`, nie usuwając historii ani znaczników `uncollected_at`.
Licznik blokady uwzględnia tylko zamówienia nieodebrane po dacie tej akcji,
więc kolejne nieodebrane zamówienie znów zablokuje konto. Dotyczy wyłącznie
kont uczniów (`400` dla innych ról); brak konta → `404`. Audyt zapisuje
`uncollected_block_cleared`.

**Odpowiedź 200**

```json
{"ok": true, "cleared_at": 1762451234}
```

### `GET /api/admin/settings`

```json
{
  "cafeteria_name": "Stołówka ZSŁ",
  "ordering_enabled": "1",
  "payment_info": "Płatność przy odbiorze: gotówka lub karta.",
  "max_open_orders": "3",
  "max_order_items": "20",
  "cancel_window_minutes": "30",
  "pickup_limit_minutes": "120",
  "max_uncollected": "0"
}
```

Wartości ustawień są stringami (tak jak w tabeli `settings`).

### `PUT /api/admin/settings`

Wszystkie pola opcjonalne; podaje się tylko zmieniane.

```json
{
  "cafeteria_name": "Stołówka ZSŁ",
  "ordering_enabled": true,
  "payment_info": "Płatność przy odbiorze: gotówka lub karta.",
  "max_open_orders": 3,
  "max_order_items": 20,
  "cancel_window_minutes": 30,
  "pickup_limit_minutes": 120,
  "max_uncollected": 0
}
```

| Pole | Walidacja |
| --- | --- |
| `cafeteria_name` | 1–80 znaków |
| `ordering_enabled` | bool → `"1"` / `"0"` |
| `payment_info` | ≤ 300 znaków |
| `max_open_orders` | 1–10 |
| `max_order_items` | 1–50 |
| `cancel_window_minutes` | 0–1440; 0 = bez terminu |
| `pickup_limit_minutes` | 0–720; 0 = auto-oznaczanie wyłączone |
| `max_uncollected` | 0–20; 0 = brak blokady |

Brak zmian → `400`. Zwraca pełny obiekt ustawień.

### `GET /api/admin/audit?limit=200`

`limit` domyślnie 200, zakres 1–500. Zwraca najnowsze wpisy:

```json
[
  {
    "id": 501,
    "action": "order_updated",
    "details": "Zamówienie id=12, status new -> preparing, płatność unpaid -> unpaid",
    "created_at": 1762451234,
    "actor_login": "k-9m2x7q4p1a"
  }
]
```

`actor_login` może być `null` (np. zdarzenie systemowe `admin_seeded`).
Wpisy starsze niż 90 dni są usuwane przy starcie aplikacji.

## Modele danych

### `UserView`

```json
{
  "login": "s-a7k2m9p4qx",
  "role": "student",
  "display_code": "UCZ-AB12C",
  "must_change_password": false
}
```

### `TokenResponse`

```json
{
  "access_token": "<JWT>",
  "token_type": "Bearer",
  "expires_in": 900,
  "user": { "login": "...", "role": "...", "display_code": "...", "must_change_password": false }
}
```

### `MenuItemView`

```json
{
  "id": 2,
  "name": "Kotlet schabowy z ziemniakami",
  "description": "Panierowany kotlet, ziemniaki, surówka",
  "price_cents": 1400,
  "category": "Dania główne",
  "allergens": "gluten, jaja, mleko",
  "available": true,
  "sort_order": 20
}
```

### `OrderView`

```json
{
  "id": 12,
  "code": "K7M2QP",
  "status": "new",
  "payment_method": "cash",
  "payment_status": "unpaid",
  "total_cents": 2800,
  "pickup_time": "12:30",
  "created_at": 1762450000,
  "updated_at": 1762450000,
  "paid_at": null,
  "paid_by": null,
  "uncollected_at": null,
  "student": "s-a7k2m9p4qx",
  "student_code": "UCZ-AB12C",
  "items": [
    {"name": "Kotlet schabowy z ziemniakami", "price_cents": 1400, "quantity": 2}
  ]
}
```

- `student` to pseudonimowy login, `student_code` to pseudonimowy kod —
  brak imion/nazwisk.
- `items` to snapshoty z momentu złożenia zamówienia (nazwa i cena nie
  zmieniają się po edycji menu).
- `status` może przyjąć także `uncollected` (zamówienie gotowe, ale
  nieodebrane w terminie); w bazie pozostaje `cancelled` z ustawionym
  `uncollected_at`.
- `paid_at` / `paid_by` — czas i pseudonimowy login osoby, która
  oznaczyła płatność; `null` dla nieopłaconych.
- `code` ma 6 wielkich liter; `qr_token` nie jest zwracany w JSON
  (dostępny wyłącznie jako obraz QR).

### `AdminUserView`

```json
{
  "id": 12,
  "login": "s-a7k2m9p4qx",
  "role": "student",
  "display_code": "UCZ-AB12C",
  "active": true,
  "must_change_password": false,
  "created_at": 1762450000,
  "last_login_at": 1762451234
}
```

### `AuditEntry`

```json
{
  "id": 501,
  "action": "login",
  "details": "Logowanie zakończone sukcesem",
  "created_at": 1762451234,
  "actor_login": "s-a7k2m9p4qx"
}
```

## Błędy

Format:

```json
{
  "error": "Czytelny komunikat po polsku.",
  "code": "bad_request"
}
```

| HTTP | `code` | Znaczenie |
| --- | --- | --- |
| 400 | `bad_request` | walidacja wejścia / nieprawidłowa operacja |
| 401 | `unauthorized` | brak sesji, wygaśnięcie, błędne dane logowania |
| 403 | `forbidden` | brak roli, błędny/brak CSRF, zablokowane pochodzenie |
| 403 | `password_change_required` | konto musi najpierw zmienić hasło startowe |
| 404 | `not_found` | brak zasobu lub brak uprawnień do zasobu (nie ujawniamy istnienia) |
| 409 | `conflict` | konflikt stanu (limit, zamknięte zamówienie, anulowane) |
| 429 | `too_many_requests` | rate limit; `Retry-After` |
| 500 | `internal_error` | błąd wewnętrzny; szczegóły nie są ujawniane |

Dodatkowo framework może zwrócić `405 Method Not Allowed` dla złej metody
oraz `413 Payload Too Large` przy ciele > 64 KiB.

Odpowiedzi błędów API mają `Cache-Control: no-store` i nie zawierają
szczegółów SQL, ścieżek ani stack trace.

## Walidacja

| Pole | Reguła |
| --- | --- |
| login | ≤ 64 znaki po przycięciu; dokładne dopasowanie |
| hasło logowania | ≤ 256 znaków |
| nowe hasło | 10–128 znaków, litera ASCII + cyfra, różne od obecnego |
| nazwa menu | 1–80 znaków |
| opis menu | ≤ 240 znaków |
| cena menu | 0–100 000 gr |
| kategoria | 1–40 znaków |
| alergeny | ≤ 160 znaków |
| `sort_order` | −10 000…10 000 |
| ilość pozycji | 1–10 |
| liczba pozycji zamówienia | 1–`max_order_items` (1–50) |
| `pickup_time` | `""` lub `HH:MM`, 00:00–23:59 |
| `idempotency_key` | 8–128 znaków `[A-Za-z0-9._:-]`, wymagane |
| `cancel_window_minutes` | 0–1440 |
| `pickup_limit_minutes` | 0–720 |
| `max_uncollected` | 0–20 |
| kwota zamówienia | ≤ 200 000 gr |
| `code` (pickup) | ≤ 128 znaków, przycinany |
| `count` generowania | 1–100 |
| `limit` audytu | 1–500 |
| `cafeteria_name` | 1–80 znaków |
| `payment_info` | ≤ 300 znaków |

Wszystkie zapytania SQL są parametryzowane (`sqlx`), a dynamiczne filtry
budowane przez `QueryBuilder` z bindowaniem wartości.

## Limity i rate limiting

- Ciało żądania: **64 KiB** (wszystkie ścieżki).
- Logowanie i wydanie tokenu JWT: 10 nieudanych prób na login i 80 na IP
  w oknie 10 minut → blokada 15 minut. Limiter działa w pamięci procesu
  i jest wspólny dla `/api/auth/login` oraz `/api/auth/token`.
- Zamówienia: maks. `max_open_orders` (1–10, domyślnie 3) otwartych na
  użytkownika; maks. `max_order_items` pozycji w zamówieniu (1–50,
  domyślnie 20).
- Listy: zamówienia personelu maks. 300, zamówienia ucznia maks. 200,
  konta maks. 500, audyt maks. 500.
- Sesja: domyślnie 12 h (`session_hours`, 1–720).
- Konta: maks. 20 000 w systemie.

## Przykład sesji curl

```bash
BASE=http://127.0.0.1:3000
JAR=/tmp/zsl.cookies

# 1. Logowanie (zapis ciasteczka)
LOGIN_JSON=$(curl -s -c "$JAR" -H 'Content-Type: application/json' \
  -d '{"login":"admin-xxxxxxxxxx","password":"StartoweHaslo1"}' \
  "$BASE/api/auth/login")

CSRF=$(printf '%s' "$LOGIN_JSON" | jq -r '.csrf_token')

# 2. Zmiana hasła startowego
curl -s -b "$JAR" -c "$JAR" -H 'Content-Type: application/json' \
  -H "X-CSRF-Token: $CSRF" \
  -d '{"current_password":"StartoweHaslo1","new_password":"NoweHaslo123"}' \
  "$BASE/api/auth/change-password"

# 3. Wygenerowanie kont uczniów
curl -s -b "$JAR" -H 'Content-Type: application/json' \
  -H "X-CSRF-Token: $CSRF" \
  -d '{"role":"student","count":5}' \
  "$BASE/api/admin/users/generate" | jq .

# 4. Przykładowe zamówienie (po zalogowaniu ucznia — inny JAR)
curl -s -b /tmp/student.cookies -H 'Content-Type: application/json' \
  -H "X-CSRF-Token: $STUDENT_CSRF" \
  -d '{"items":[{"menu_item_id":2,"quantity":1}],"payment_method":"cash","pickup_time":"12:30"}' \
  "$BASE/api/student/orders" | jq .

# 5. Wariant API: token JWT i żądanie Bearer (bez CSRF)
TOKEN_JSON=$(curl -s -H 'Content-Type: application/json' \
  -d '{"login":"s-a7k2m9p4qx","password":"TwojeHaslo123"}' \
  "$BASE/api/auth/token")
TOKEN=$(printf '%s' "$TOKEN_JSON" | jq -r '.access_token')

curl -s -H "Authorization: Bearer $TOKEN" \
  "$BASE/api/student/orders" | jq .

curl -s -X POST -H "Authorization: Bearer $TOKEN" \
  "$BASE/api/auth/logout"
# od tej chwili token zwraca 401 (sesja JWT usunięta)
```

Po restarcie serwera limiter logowania jest pusty, a sesje pozostają ważne
do czasu wygaśnięcia (są w bazie). Tokeny JWT pozostają ważne do `exp`
lub do unieważnienia; usunięcie `jwt.key` unieważnia wszystkie tokeny.
