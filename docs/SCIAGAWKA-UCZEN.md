# Ściągawka ucznia — ZSL Order

**Panel:** `http://<ADRES-SYSTEMU>:3000/student` (albo adres podany przez szkołę)

## 1. Pierwsze logowanie

1. Wpisz login i hasło z karty.
2. System poprosi o nowe hasło. Wpisz je dwa razy.
3. Nowe hasło: **minimum 10 znaków**, nie może zawierać loginu ani być
   popularnym hasłem (np. `qwerty123`).
4. Zapamiętaj hasło. Nie masz odzyskiwania przez e-mail.

## 2. Zamówienie

1. Wybierz pozycje z menu i ilości.
2. Sprawdź alergeny i cenę.
3. Przy pozycjach limitowanych widzisz **„Zostało N szt.”**;
   **„Wyprzedane na dziś”** oznacza, że nie można jej już zamówić.
4. Wybierz płatność: **gotówka** albo **karta**.
5. Opcjonalnie podaj godzinę odbioru `HH:MM`.
6. Kliknij **Zamów**.

Jeśli w trakcie zamawiania ktoś kupi ostatnią sztukę, zobaczysz
komunikat „Skończyły się: …”. Odśwież menu i spróbuj ponownie.

Po zamówieniu dostaniesz:

- **krótki kod** (np. `K7M2QP`),
- **kod QR**.

## 3. Odbiór

1. Przyjdź po zamówienie w wyznaczonym czasie.
2. Pokaż kod QR albo podaj krótki kod.
3. Kuchnia wyda posiłek i oznaczy płatność.

Płatność następuje przy odbiorze — zgodnie z wyborem w zamówieniu.

## 4. Anulowanie

Możesz anulować tylko:

- zanim kuchnia wyda posiłek (status `ordered`),
- w terminie ustalonym przez szkołę (domyślnie 30 minut),
- gdy zamówienie nie jest jeszcze opłacone.

Po anulowaniu kod QR przestaje działać. Jeśli zamówienie złożono
dzisiaj i nie zostało jeszcze wydane, anulowanie zwraca sztukę na stan
menu. Jeśli przycisk jest nieaktywny, poproś kuchnię o pomoc.

## 5. Statusy

| Status | Co to znaczy |
| --- | --- |
| `ordered` | przyjęte, czeka na wydanie |
| `issued` | wydane |
| `cancelled` | anulowane |
| `uncollected` | nieodebrane |

## 6. Zasady bezpieczeństwa

- Nie podawaj loginu i hasła kolegom.
- Nie publikuj kodu QR w internecie.
- QR jest jednorazowy i działa tylko dla dzisiejszego zamówienia.
- Nie zapisuj hasła na kartce przyklejonej do komputera.

## 7. Problemy

| Problem | Co zrobić |
| --- | --- |
| Nie pamiętam hasła | Poproś administratora/stołówkę o reset hasła. |
| Konto zablokowane | Odczekaj 15 minut albo poproś administratora. |
| Nie widzę zamówienia | Sprawdź, czy jesteś zalogowany na swoje konto. |
| QR nie działa | Sprawdź, czy zamówienie jest `ordered` i z dzisiaj; po wydaniu kod jest zużyty. |
| Pozycja jest wyprzedana | Wybierz inną pozycję albo poproś kuchnię o uzupełnienie stanu. |
| Zła godzina/cena | Zgłoś kuchni przed wydaniem. |

**Nie podawaj nikomu kodu QR przez telefon ani w wiadomości.**
