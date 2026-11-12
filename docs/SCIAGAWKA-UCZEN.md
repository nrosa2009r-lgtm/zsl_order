# Ściągawka ucznia — ZSL Order

**Panel:** `http://<ADRES-SERWERA>:3000/student` (albo adres podany przez szkołę)

## 1. Pierwsze logowanie

1. Wpisz login i hasło z karty.
2. System poprosi o nowe hasło. Wpisz je dwa razy.
3. Nowe hasło: **minimum 12 znaków, litera i cyfra**.
4. Zapamiętaj hasło. Nie masz odzyskiwania przez e-mail.

## 2. Zamówienie

1. Wybierz pozycje z menu i ilości.
2. Sprawdź alergeny i cenę.
3. Wybierz płatność: **gotówka** albo **karta**.
4. Opcjonalnie podaj godzinę odbioru `HH:MM`.
5. Kliknij **Zamów**.

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

- zanim kuchnia zacznie przygotowanie (status `new`),
- w terminie ustalonym przez szkołę (domyślnie 30 minut).

Po anulowaniu kod QR przestaje działać. Jeśli przycisk jest nieaktywny,
poproś kuchnię o pomoc.

## 5. Statusy

| Status | Co to znaczy |
| --- | --- |
| `new` | przyjęte, czeka |
| `preparing` | w przygotowaniu |
| `ready` | gotowe do odbioru |
| `completed` | wydane |
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
| QR nie działa | Sprawdź, czy zamówienie jest `ready` i z dzisiaj; po wydaniu kod jest zużyty. |
| Zła godzina/cena | Zgłoś kuchni przed wydaniem. |

**Nie podawaj nikomu kodu QR przez telefon ani w wiadomości.**
