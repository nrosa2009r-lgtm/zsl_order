# Ściągawka kuchni — ZSL Order

**Panel:** `http://<ADRES-SYSTEMU>:3000/kitchen` (albo adres HTTPS)

## 1. Start zmiany

1. Zaloguj się kontem `kitchen`.
2. Zmiana hasła: minimum **12 znaków**, bez loginu i popularnych haseł
   (przycisk **Zmień hasło**).
3. Sprawdź zakładkę **Zamówienia** — widoczne są zamówienia `ordered`.
4. Sprawdź, czy tablet ma poprawną godzinę i strefę czasu.
5. Przygotuj czytnik USB/Bluetooth albo sprawdź, czy kamera działa.

## 2. Statusy

```text
ordered → issued
```

- `ordered` — przyjęte, czeka na wydanie,
- `issued` — wydane (automatycznie opłacone, jeśli było nieopłacone),
- `cancelled` / `uncollected` — zamknięte, kuchnia ich nie zmienia.

Karta zamówienia pokazuje kod ucznia. Po usunięciu konta ucznia widnieje
tam **„konto usunięte”** — to normalne; zamówienie pozostało w historii
i dalej trafia do raportu.

Kuchnia zmienia tylko **dzisiejsze** zamówienia. Zamówienie na przyszły
dzień ma na karcie datę odbioru i informację „wydanie będzie możliwe
w dniu odbioru” — nie wydawaj go wcześniej i nie oznaczaj jako
nieodebrane.

## 3. Wydanie QR — 3 kroki

1. Zakładka **Odbiór QR** — pole kodu ma fokus.
2. Zeskanuj QR, użyj czytnika USB (Enter = szukaj) albo wpisz kod ręcznie.
3. Sprawdź kwotę i płatność, kliknij duży przycisk **Wydaj**.

Po sukcesie: dźwięk, wibracja i powrót fokusu — skanuj następne
zamówienie bez klikania.

## 4. Płatność

- Zamówienie nieopłacone → **Wydaj** oznacza je jako opłacone.
- Zamówienie opłacone → informacja „Opłacone”.
- Cofnąć płatność może **tylko administrator**.

## 5. Błędy przy skanie

| Kod | Znaczenie | Co zrobić |
| --- | --- | --- |
| `404` | nie ma takiego kodu | sprawdź literówkę / zeskanuj ponownie |
| `409` | zły dzień, już wydane, anulowane, nieodebrane | przeczytaj komunikat |
| `429` | limit błędnych prób | odczekaj 15 minut |

Czerwona karta = błąd. Pole kodu zostaje aktywne.

## 6. Kamera nie działa

Kamera wymaga **HTTPS albo localhost**. Po HTTP na tablecie nie
zadziała.

Kolejność rozwiązań:

1. Czytnik USB/Bluetooth (działa jak klawiatura),
2. wpisanie kodu ręcznie,
3. HTTPS w sieci lokalnej,
4. tryb `tunnel` (Cloudflare) — tylko z Access.

## 7. Nieodebrane

- Zamówienie staje się `uncollected` po **końcu swojej przerwy +
  `pickup_limit_minutes`**. Nie licz tego od godziny złożenia — poranne
  zamówienie na późniejszą przerwę ma czas do końca tej przerwy.
- Zamówienia z poprzednich dni są zamykane zawsze; dzisiejsze bez przerwy
  (stare dane) nie są zamykane automatycznie.
- Kuchnia nie zmienia takiego zamówienia.
- Blokadę u ucznia zdejmuje **administrator**.

## 7a. Przerwy i dzień odbioru

- Karta zamówienia pokazuje **dzień odbioru** i **przerwę** (np. „Długa
  przerwa”). Wydawaj zgodnie z nimi.
- Kod QR i przycisk **Wydaj** są aktywne wyłącznie w dniu odbioru.
- Automatyczne „nieodebrane” pomija zamówienia, których dzień odbioru
  jeszcze nie nadszedł.
- W raporcie dnia tabela **Przerwy** pokazuje liczbę zamówień, wydanych,
  opłaconych i nieodebranych dla każdego okna.

## 8. Stan magazynowy

- Karta pozycji w **Menu** pokazuje **„Zostało N szt.”** albo
  **„Bez limitu”**.
- Szybkie przyciski **−1 szt.**, **+1 szt.** i **Bez limitu** zmieniają
  stan od razu; każda zmiana trafia do audytu.
- W dialogu pozycji ustaw **Stan na dziś** (ile porcji zostało) i
  opcjonalnie **Domyślny stan dzienny**:
  - puste „Stan na dziś” = bez limitu,
  - puste „Domyślny stan” = brak automatycznego resetu.
- Nowy dzień: pozycje z domyślnym stanem wracają do niego przy pierwszym
  odczycie menu. Pozostałe zachowują ostatni stan — sprawdź je rano.
- Gdy stan spadnie do zera, uczeń widzi **„Wyprzedane na dziś”**.
- Anulowanie dzisiejszego zamówienia przed wydaniem zwraca sztukę.
- Bieżący stan i sprzedaż dnia znajdziesz w zakładce **Raport**.

## 9. Koniec zmiany

- [ ] Wszystkie wydane zamówienia są `issued`.
- [ ] Nie ma otwartych `ordered` bez wyjaśnienia.
- [ ] Stan magazynowy na jutro ustawiony (albo `Domyślny stan dzienny`).
- [ ] Tablet wylogowany lub zablokowany.
- [ ] Czytnik odłożony, karta z hasłem schowana.
- [ ] Problemy zgłoszone administratorowi.

**Nie zostawiaj zalogowanego panelu bez nadzoru.**
