# Prywatność, dane i RODO — ZSL Order

Ten dokument opisuje, jakie dane zbiera aplikacja, w jakim celu, jak długo
je przechowuje i jakie środki stosuje. Ma charakter techniczno-organizacyjny
— **nie jest opinią prawną ani gwarancją pełnej zgodności z RODO**.
Ostateczną ocenę (podstawa prawna, obowiązek informacyjny, DPIA, umowy
powierzenia) musi wykonać szkoła jako administrator danych.

## 1. Zasada projektowa: minimalizacja i pseudonimizacja

System został zaprojektowany tak, aby **nie zbierać danych osobowych
uczniów**:

- brak rejestracji i formularzy osobowych,
- brak imion, nazwisk, adresów e-mail, telefonów, adresów, klasy, daty
  urodzenia, zdjęć,
- uczeń otrzymuje losowy login i losowe hasło,
- w systemie występuje pod pseudonimowym kodem (`display_code`),
- płatność odbywa się przy ladzie — aplikacja nie widzi danych karty,
- QR zawiera tylko losowy token.

To jest **pseudonimizacja**, a nie anonimizacja w rozumieniu prawnym:
jeśli szkoła posiada gdzieś (np. na wydruku, w dzienniku, w arkuszu)
powiązanie loginu z konkretnym uczniem, to po stronie szkoły mogą powstać
dane osobowe. System tego mapowania nie przechowuje i nie wymaga.

Wyjątkiem jest import CSV w formacie `klasa;liczba`: klasa/grupa trafia
do loginu (`s-1a-...`) i kodu konta (`UCZ-1A-...`). To **zmniejsza
pseudonimizację** — osoba z dostępem do listy kont może zawęzić krąg
uczniów do jednej klasy, nawet bez imion i nazwisk. Dlatego import nie
przyjmuje imion, nazwisk ani numerów z dziennika, klasa jest krótka i nie
identyfikuje konkretnej osoby, a dostęp do panelu kont i eksportu CSV ma
wyłącznie administrator. Mapowanie login → uczeń nadal powinno być
przechowywane poza systemem.

## 2. Jakie dane są przechowywane

### `users`

| Dane | Cel | Uwagi |
| --- | --- | --- |
| losowy `login` | identyfikacja konta | bez związku z tożsamością |
| `password_hash` (Argon2id) | uwierzytelnianie | hasło jawne nigdy nie jest zapisywane |
| `role` | uprawnienia | student/kitchen/admin |
| `display_code` | pseudonim w UI | losowy kod |
| `active` | blokada konta | 0/1 |
| `must_change_password` | wymuszona zmiana hasła | 0/1 |
| `created_at` | techniczny czas utworzenia | UTC |
| `last_login_at` | techniczny czas logowania | UTC, opcjonalny |

### `sessions`

| Dane | Cel | Uwagi |
| --- | --- | --- |
| `token_hash` | utrzymanie sesji | tylko SHA-256 tokenu |
| `user_id` | powiązanie z kontem | techniczne |
| `csrf_token` | ochrona CSRF | losowy |
| `created_at`, `expires_at`, `last_seen_at` | ważność sesji | UTC |

### `orders` i `order_items`

| Dane | Cel | Uwagi |
| --- | --- | --- |
| `user_id` | przypisanie zamówienia | pseudonimowe konto |
| `code` | odbiór zamówienia | 6 liter |
| `qr_token` | QR odbioru | losowy token |
| `status`, `payment_method`, `payment_status` | realizacja | brak danych karty |
| `total_cents` | kwota | grosze |
| `pickup_time` | godzina odbioru | `HH:MM` |
| `created_at`, `updated_at` | porządek i statystyki | UTC |
| `name`, `price_cents`, `quantity` w `order_items` | historia zamówienia | snapshot menu |

### `settings`

Konfiguracja stołówki: nazwa, informacja o płatności, limity. Brak danych
osobowych.

### `audit_log`

| Dane | Cel | Uwagi |
| --- | --- | --- |
| `actor_id` | kto wykonał akcję | techniczny ID konta lub NULL |
| `action` | rodzaj zdarzenia | np. `login`, `order_updated` |
| `details` | techniczny opis | ID zamówienia, statusy, kwota |
| `created_at` | czas | UTC |

Audyt nie zawiera imion, adresów e-mail, tokenów sesji ani haseł.

### Czego system NIE przechowuje

- imienia i nazwiska, klasy, numeru w dzienniku,
- adresu e-mail, telefonu, adresu zamieszkania,
- daty urodzenia, PESEL, zdjęcia, podpisu,
- danych karty płatniczej lub rachunku bankowego,
- adresów IP w bazie (limiter działa w pamięci),
- nagłówka `User-Agent` ani access logów HTTP,
- treści czatów/uwag (brak takiej funkcji),
- plików cookies innych niż sesyjne.

## 3. Cele przetwarzania

1. Uwierzytelnianie i utrzymanie sesji.
2. Przyjmowanie i realizacja zamówień.
3. Wydanie posiłku na podstawie kodu/QR.
4. Rozliczenie płatności przy ladzie (tylko status, bez danych karty).
5. Bezpieczeństwo: audyt zdarzeń, limity, wykrywanie nadużyć.
6. Statystyki sprzedaży i organizacja pracy stołówki.

Nie stosujemy profilowania reklamowego, marketingu, przekazywania danych
podmiotom trzecim ani sprzedaży danych.

## 4. Podstawa prawna i role

- **Administrator danych**: szkoła/placówka prowadząca stołówkę.
- **Dostawca systemu / hosting**: ewentualny procesor — wymaga umowy
  powierzenia przetwarzania (art. 28 RODO), jeśli ma dostęp do bazy.
- **Podstawa prawna**: do ustalenia przez szkołę (np. umowa o świadczenie
  usług stołówkowych, zadanie publiczne, prawnie uzasadniony interes
  związany z bezpieczeństwem). System sam nie przesądza podstawy.
- **Obowiązek informacyjny**: szkoła powinna poinformować uczniów/
  rodziców o zasadach zamawiania, zakresie danych, retencji i prawach —
  nawet jeśli system nie zbiera danych osobowych, samo konto i zamówienie
  mogą być powiązane z uczniem poza systemem.

## 5. Pseudonimizacja i ryzyko re-identyfikacji

- Login i `display_code` są losowe; nie wynikają z imienia, klasy ani
  numeru.
- Ten sam uczeń może mieć inne konto w kolejnym roku — system nie wymaga
  trwałego identyfikatora.
- Ryzyko re-identyfikacji powstaje, gdy szkoła przechowuje mapowanie
  login → uczeń w sposób dostępny dla osób trzecich. Zalecenia:
  - przechowywać mapowanie poza systemem, w miejscu o ograniczonym
    dostępie,
  - nie umieszczać imion w nazwach menu, opisach ani w polu `payment_info`,
  - nie wysyłać list loginów/haseł na grupowe czaty,
  - nadawać unikalne konta i odbierać je po zakończeniu roku.
- QR i krótki kod nie zawierają danych osobowych, ale są **bearer
  tokenami** — kto je zna, może podszyć się przy odbiorze. Nie publikuj
  ich.

## 6. Retencja

System ma wbudowane nocne czyszczenie (domyślnie o 03:30 czasu szkoły,
nigdy przy starcie; porcje po 1000 rekordów):

- wygasłe sesje (ciasteczkowe i JWT) — zawsze,
- klucze idempotencji starsze niż `RETENTION_IDEMPOTENCY_DAYS`
  (domyślnie 7 dni) są zerowane, ale same zamówienia zostają,
- audyt starszy niż `RETENTION_AUDIT_DAYS` (domyślnie 90 dni),
- zamówienia starsze niż `RETENTION_ORDERS_DAYS`; **domyślnie 0, czyli
  wyłączone** — przy starcie pojawia się wtedy ostrzeżenie w logu.

Poza tym **nie ma automatycznej retencji kont**. Szkoła musi
ustalić politykę, np.:

| Dane | Rekomendacja |
| --- | --- |
| sesje | maks. `session_hours` (domyślnie 12 h); JWT 15 min |
| audyt | 90 dni (`RETENTION_AUDIT_DAYS`) |
| zamówienia i pozycje | `RETENTION_ORDERS_DAYS` (domyślnie 0 = brak usuwania); rok szkolny lub krócej |
| konta uczniów | do końca roku szkolnego; potem dezaktywuj i usuń |
| konta kuchni/admina | przez okres zatrudnienia/umowy |
| backupy | krótko, zgodnie z polityką; szyfrowane i ograniczone |

Przykładowe zapytania retencji: [DATABASE.md](DATABASE.md#6-retencja-i-porządki).

## 7. Prawa osób

Jeśli szkoła potrafi powiązać konto z uczniem, muszą być zapewnione
procedury:

- **dostęp** — wydanie danych konta i zamówień (administrator może
  wyeksportować je z bazy),
- **sprostowanie** — korekta danych poza systemem; w systemie można
  zmienić hasło, kod, aktywność,
- **usunięcie** — dezaktywacja i usunięcie konta oraz rozstrzygnięcie
  historii zamówień (`orders.user_id` ma `ON DELETE RESTRICT`),
- **ograniczenie** — dezaktywacja konta,
- **sprzeciw** — do rozpatrzenia przez szkołę,
- **przenoszenie** — eksport techniczny z bazy (CSV/JSON) po stronie
  administratora.

System nie ma panelu „prawa osoby” — realizacja odbywa się przez
administratora i dostęp do bazy, z zachowaniem backupu i audytu.

## 8. Bezpieczeństwo danych

Środki techniczne:

- hasła Argon2id, tokeny sesji tylko jako SHA-256,
- ciasteczka `HttpOnly`, `SameSite=Strict`, `Secure` przy HTTPS,
- CSRF, RBAC, rate limiting, walidacja, parametryzowany SQL,
- CSP bez inline, `frame-ancestors 'none'`, `nosniff`, `no-referrer`,
- minimalizacja pól i retencja audytu,
- brak danych kart płatniczych,
- plik bazy i `config.json` z uprawnieniami `0600` (zalecane),
- backup traktowany jak dane wrażliwe.

Środki organizacyjne (po stronie szkoły):

- dostęp do serwera tylko dla uprawnionych,
- osobne konta admina, silne hasła, ograniczenie sieci,
- procedura przekazywania loginów/haseł,
- polityka retencji i backupów,
- rejestr czynności przetwarzania (RCPD), jeśli dotyczy,
- ocena skutków dla ochrony danych (DPIA), jeśli wymagana,
- zgłaszanie incydentów zgodnie z procedurą (72 h do organu, jeśli
  wystąpi naruszenie).

## 9. Przekazywanie danych

- Aplikacja nie wysyła danych do zewnętrznych usług, CDN, analityki ani
  reklam.
- Wszystkie zasoby (HTML/CSS/JS/QR) są generowane/serwowane lokalnie.
- Jeśli hosting jest zewnętrzny, dostawca może być procesorem — wymagana
  umowa i ocena lokalizacji danych.
- Backupy mogą być przechowywane poza serwerem — również podlegają
  zabezpieczeniom i umowom.

## 10. QR, kody i logi

- QR: `ZSL:<48 hex>` — losowy token, bez danych ucznia. Traktuj jak bilet.
- Krótki kod: 6 liter, wygoda personelu; nie publikuj.
- Logi aplikacji: **JSON w journald**, bez plików `logs/`. Zawierają
  metodę, ścieżkę bez query, status i czas odpowiedzi oraz techniczne
  komunikaty (np. `account_id`, rola). **Nie** zawierają nagłówków,
  ciasteczek, tokenów JWT/CSRF, haseł ani ciał żądań. Adres IP występuje
  wyłącznie w zdarzeniach bezpieczeństwa (nieudane logowanie, blokada
  limitera); zamiast loginu zapisywane jest ID konta. Ustaw retencję
  journald na 30 dni (`MaxRetentionSec=30day`) i limit rozmiaru
  (`SystemMaxUse`), zgodnie z [DEPLOYMENT.md](DEPLOYMENT.md).
- Logi proxy (jeśli włączone): zawierają IP i ścieżki — ustal retencję,
  ogranicz dostęp, nie loguj ciał żądań.
- Audyt: techniczny, 90 dni; może być kopiowany do systemu append-only.

## 11. Rekomendowany tekst informacyjny dla uczniów

Przykład (do dostosowania przez szkołę):

> System zamawiania posiłków nie przechowuje Twojego imienia, nazwiska,
> adresu e-mail ani danych karty. Otrzymujesz losowy login i hasło.
> W systemie widniejesz pod pseudonimowym kodem. Zamówienie zawiera
> pozycje, kwotę, sposób płatności przy ladzie, godzinę odbioru i czas
> złożenia. Kod QR służy tylko do odbioru posiłku — nie udostępniaj go
> osobom postronnym. Dane zamówień są przechowywane przez [okres] i
> usuwane po [okres]. W sprawie dostępu lub usunięcia danych skontaktuj
> się z [administrator/sekretariat].

## 12. Checklista prywatności przed wdrożeniem

- [ ] Ustalona podstawa prawna i zakres danych.
- [ ] Ustalona retencja zamówień, kont, backupów i logów.
- [ ] Procedura przekazywania loginów/haseł (bez e-maili grupowych).
- [ ] Mapowanie login → uczeń (jeśli istnieje) poza systemem, z kontrolą
      dostępu.
- [ ] Informacja dla uczniów/rodziców.
- [ ] Umowa powierzenia z hostingiem (jeśli dotyczy).
- [ ] DPIA, jeśli wymagana.
- [ ] Procedura realizacji praw i zgłaszania incydentów.
- [ ] Backup szyfrowany i testowany.
- [ ] Dostęp do bazy ograniczony do minimum.
- [ ] Brak imion i danych kontaktowych w menu, ustawieniach i audycie.

## 13. Ważne zastrzeżenie

Minimalizacja danych i pseudonimizacja **zmniejszają ryzyko**, ale nie
eliminują wszystkich obowiązków. To, czy dane są „anonimowe” w rozumieniu
RODO, zależy od możliwości ponownej identyfikacji w konkretnym środowisku
(m.in. od tego, czy szkoła przechowuje mapowanie kont). Decyzję i
dokumentację zgodności podejmuje administrator danych — szkoła.
