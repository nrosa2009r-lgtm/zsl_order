# Ściągawka kuchni — ZSL Order

**Panel:** `http://<ADRES-SERWERA>:3000/kitchen` (albo adres HTTPS)

## 1. Start zmiany

1. Zaloguj się kontem `kitchen`.
2. Sprawdź zakładkę **Zamówienia** — widoczne są `new`, `preparing`,
   `ready`.
3. Sprawdź, czy tablet ma poprawną godzinę i strefę czasu.
4. Przygotuj czytnik USB/Bluetooth albo sprawdź, czy kamera działa.

## 2. Statusy

```text
new → preparing → ready → completed
```

- `new` — przyjęte,
- `preparing` — w przygotowaniu,
- `ready` — gotowe, można wydać,
- `completed` — wydane (automatycznie opłacone, jeśli było nieopłacone),
- `cancelled` / `uncollected` — zamknięte, kuchnia ich nie zmienia.

Kuchnia zmienia tylko **dzisiejsze** zamówienia.

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
| `409` | zły dzień, nie `ready`, już wydane, anulowane, nieodebrane | przeczytaj komunikat |
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

- Po czasie `pickup_limit_minutes` zamówienie staje się `uncollected`.
- Kuchnia nie zmienia takiego zamówienia.
- Blokadę u ucznia zdejmuje **administrator**.

## 8. Koniec zmiany

- [ ] Wszystkie wydane zamówienia są `completed`.
- [ ] Nie ma otwartych `ready` bez wyjaśnienia.
- [ ] Tablet wylogowany lub zablokowany.
- [ ] Czytnik odłożony, karta z hasłem schowana.
- [ ] Problemy zgłoszone administratorowi.

**Nie zostawiaj zalogowanego panelu bez nadzoru.**
