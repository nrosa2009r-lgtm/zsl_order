# Checklista wdrożenia — ZSL Order

Dokument do odhaczania przed pilotażem, przed produkcją i przed
włączeniem tunelu. Każdy punkt powinien mieć **właściciela, datę i
dowód** (zrzut ekranu, wynik testu, wpis w rejestrze zmian). Nie wpisuj
tu prawdziwych haseł, kluczy, adresów IP ani danych uczniów — używaj
placeholderów i menedżera haseł.

Legenda: `[ ]` do zrobienia, `[x]` zrobione, `N/D` nie dotyczy (opisz
dlaczego).

## A. Przed pilotażem na Dellu (tryb `local`, port 80, czytnik USB)

### A1. Decyzje, zgody i odpowiedzialność

- [ ] Dyrekcja szkoły zatwierdziła zakres pilotażu, czas trwania i
      kryterium sukcesu (np. 2 tygodnie, jedna klasa/stołówka).
- [ ] Inspektor Ochrony Danych (IOD) oraz osoba odpowiedzialna za
      ochronę danych w szkole widzieli opis systemu i nie zgłosili
      zastrzeżeń; ustalono, czy wymagana jest ocena skutków dla ochrony
      danych (DPIA).
- [ ] Jest **opiekun po stronie szkoły**: imię/nazwisko lub rola,
      telefon służbowy, zastępca i procedura eskalacji.
- [ ] Ustalono, kto ma dostęp do panelu administratora, kto do kuchni
      i kto może zmieniać konfigurację serwera.
- [ ] Potwierdzono na piśmie zasadę minimalizacji danych: brak imion,
      nazwisk, e-maili, telefonów, PESEL, numerów kart i adresów
      uczniów w systemie.
- [ ] Ustalono politykę retencji: zamówienia, audyt, kopie zapasowe,
      logi journald.

### A2. Sprzęt, sieć i stanowisko

- [ ] Dell przygotowany: aktualny system, UPS, stabilne zasilanie,
      zamknięta szafa lub pomieszczenie serwerowe.
- [ ] Stały adres w sieci szkolnej (rezerwacja DHCP lub adres stały).
- [ ] Aplikacja dostępna na **porcie 80** w sieci szkolnej. Port 80
      obsługuje reverse proxy (Nginx/Caddy) albo systemd z
      `CAP_NET_BIND_SERVICE`; usługa **nie działa jako root**.
- [ ] Firewall dopuszcza port 80 wyłącznie z podsieci szkolnej; brak
      przekierowania portu z internetu.
- [ ] Czytnik USB/Bluetooth (klawiaturowy) przetestowany na komputerze
      kuchni: wpisuje kod i wysyła `Enter`.
- [ ] Stanowisko kuchni: komputer/tablet, aktualna przeglądarka,
      czysty ekran, brak dostępu osób postronnych do zalogowanej sesji.
- [ ] Telefon/tablet do testów panelu ucznia; aparat nie jest wymagany
      do pilotażu, jeśli używany jest czytnik USB.
- [ ] Strefa czasu `Europe/Warsaw` i synchronizacja czasu (NTP)
      działają; data i godzina na Dellu są poprawne.

### A3. Instalacja, konta, hasła i klucze

- [ ] Instalacja z builda `cargo build --release --locked`; zapisana
      wersja/commit i suma kontrolna binarki.
- [ ] Konto systemowe `zsl-order`, katalogi `/opt/zsl-order`,
      `/var/lib/zsl-order` (`0700`) i `/etc/zsl-order` (`0750`).
- [ ] Plik środowiskowy `/etc/zsl-order/zsl-order.env` ma `0640`,
      należy do `root:zsl-order` i **nie zawiera** prawdziwych haseł.
- [ ] `DEPLOY_MODE=local`, `BIND_ADDR` ustawiony świadomie,
      `COOKIE_SECURE=auto`; `PUBLIC_URL` zgodny z adresem pilotażu.
- [ ] `jwt.key` utworzony automatycznie, uprawnienia `0600`; nie ma go
      w repozytorium ani w kopii udostępnianej przez czat.
- [ ] `admin_start.txt` ma `0600`, został odebrany, a po pierwszej
      zmianie hasła usunięty; sprawdzono, że nie jest serwowany przez
      HTTP.
- [ ] Hasło administratora zapisane w menedżerze haseł; istnieje
      drugie konto administratora awaryjnego.
- [ ] Hasła kont kuchni i uczniów są unikalne, losowe i przekazywane
      wyłącznie kartką/CSV według procedury z instrukcji.
- [ ] Sprawdzono uprawnienia plików: baza, `-wal`, `-shm`, `jwt.key`,
      `admin_start.txt`, katalog backupów — bez dostępu dla
      nieuprawnionych.
- [ ] Klucze i sekrety nie trafiły do Git, czatu, e-maila, chmury ani
      logów.

### A4. Backup, retencja i logi

- [ ] Katalog backupów istnieje, ma `0700` i wystarczające miejsce.
- [ ] Backup działa o `03:15` (z losowym opóźnieniem do 15 minut), a
      retencja o `04:00`; kolejność i godziny są zgodne z konfiguracją.
- [ ] Wykonano **test odtworzenia backupu na drugiej maszynie**:
      kopia otwiera się, `PRAGMA integrity_check` = `ok`, logowanie i
      odczyt zamówień działają.
- [ ] Ustalono retencję kopii i sposób ich szyfrowania; kopie poza
      serwerem są zaszyfrowane i dostępne tylko dla uprawnionych.
- [ ] Journald ma ustawione `SystemMaxUse` i `MaxRetentionSec`
      (np. 30 dni); logi nie zawierają haseł, tokenów ani ciał żądań.
- [ ] Sprawdzono, że `RETENTION_ORDERS_DAYS` nie jest przypadkowo `0`
      bez decyzji szkoły.

### A5. Testy funkcjonalne i bezpieczeństwa

- [ ] `GET /api/healthz` i `/healthz` zwracają `200`.
- [ ] Logowanie ucznia, kuchni i admina; wymuszona zmiana hasła przy
      pierwszym logowaniu; reset hasła; unieważnienie sesji.
- [ ] Import/generowanie kont, pobranie CSV z hasłami, ostrzeżenie
      „Hasła nie będą dostępne po wyjściu”, brak ponownego pobrania po
      opuszczeniu widoku.
- [ ] Po teście CSV usunięty z komputera, folderu **Pobrane** i kosza;
      sprawdzono, że hasła nie trafiły do audytu ani logów.
- [ ] Wydruk kart jednostronny w zamkniętym pomieszczeniu; wydawanie
      przez wychowawców; zniszczenie niewydanych kart.
- [ ] Zamówienie ucznia, zmiana statusów, odbiór QR tylko dla
      dzisiejszego `ready`, jednorazowość, czytnik USB i wpis ręczny.
- [ ] Kamera na HTTP pokazuje komunikat o HTTPS/localhost, a nie
      udaje, że działa; czytnik USB jest alternatywą.
- [ ] Menu, raport dnia, eksport metadanych CSV, ustawienia i audyt.
- [ ] **Test obciążeniowy** dla spodziewanego szczytu (np. 200–1000
      uczniów, przerwa obiadowa): zdefiniowane kryterium (np. p95 < 1 s,
      brak błędów 5xx, stabilna baza) i wynik zapisany.
- [ ] **Skan OWASP ZAP** (baseline + test uwierzytelniony, jeśli możliwy)
      na pilocie: brak podatności wysokich/krytycznych; znaleziska
      średnie mają właściciela i termin.
- [ ] **Test z kilku telefonów** (Android/iOS, różne przeglądarki):
      logowanie, menu, zamówienie, QR, wylogowanie.
- [ ] **Plan awaryjny kuchni (papier)**: wydruk listy zamówień/kodów,
      procedura ręcznego wydawania, kontakt do opiekuna, sposób
      odtworzenia systemu.
- [ ] **Wyłącznik awaryjny**: przećwiczono unieważnienie sesji
      (`DELETE FROM sessions` lub panel), dezaktywację konta, zatrzymanie
      usługi i unieważnienie `jwt.key`; opisano skutki i czas reakcji.
- [ ] **Zgłaszanie naruszeń**: znana ścieżka przez szkołę do IOD i
      organu nadzorczego, termin **72 godzin**, szablon zgłoszenia,
      rejestr incydentów i osoby kontaktowe.
- [ ] Sprawdzono brak danych osobowych w bazie, audycie, logach i
      plikach tymczasowych.
- [ ] Decyzja go/no-go pilotażu podpisana przez opiekuna i dyrekcję.

## B. Przed produkcją dla całej szkoły

### B1. Ponowne potwierdzenie podstaw

- [ ] Wszystkie punkty z sekcji A są zamknięte lub świadomie
      zaktualizowane dla całej szkoły.
- [ ] Dyrekcja i IOD zatwierdzili produkcję, zakres danych, retencję i
      sposób informowania uczniów.
- [ ] Opiekun po stronie szkoły i zastępstwo są formalnie wyznaczeni;
      znany jest plan urlopów i eskalacji.
- [ ] Zaktualizowano rejestr czynności przetwarzania / dokumentację
      prywatności; uczniowie i rodzice otrzymali wymaganą informację.

### B2. Skala, wydajność i niezawodność

- [ ] Baza i konta przygotowane dla wszystkich klas; sprawdzono limity
      (20 000 kont, 200 kont na import, 100 na grupę).
- [ ] **Test obciążeniowy całej szkoły** wykonany na docelowej maszynie
      lub równoważnej; wynik i wąskie gardła opisane.
- [ ] Pojemność dysku, backupów i journald wystarcza na cały rok
      szkolny; monitorowanie zajętości miejsca działa.
- [ ] UPS, restart usługi, odtworzenie po awarii i czas powrotu (RTO)
      są przetestowane.
- [ ] Backup i **test odtworzenia na drugiej maszynie** powtórzony dla
      danych produkcyjnych; zdefiniowano RPO/RTO.
- [ ] Kopie offsite są szyfrowane, a klucz do nich jest przechowywany
      oddzielnie od kopii.

### B3. Bezpieczeństwo i prywatność

- [ ] Pełny **skan OWASP ZAP** (lub równoważny) na produkcji: brak
      podatności wysokich/krytycznych; średnie zaakceptowane pisemnie.
- [ ] Testy RBAC: uczeń i kuchnia nie mają dostępu do endpointów
      administracyjnych (403), w tym do generowania/importu i eksportu.
- [ ] Sprawdzono, że hasła startowe nie występują w odpowiedziach
      innych endpointów, audycie, logach, localStorage, sessionStorage
      ani IndexedDB.
- [ ] Polityka haseł i resetów jest znana; wymuszona zmiana hasła przy
      pierwszym logowaniu działa.
- [ ] Obrót sekretów (`jwt.key`, hasła administracyjne, ewentualne
      klucze Cloudflare) zaplanowany i przećwiczony.
- [ ] Dostęp do bazy, backupów i panelu admina ograniczony do minimum;
      konta administracyjne imienne, nie wspólne.
- [ ] Zależności: `cargo audit`, `cargo deny`, przegląd licencji i
      aktualizacja bezpieczeństwa mają właściciela i termin.
- [ ] Plan aktualizacji aplikacji i rollbacku jest opisany; wersja
      binarki i frontendu jest identyfikowalna.

### B4. Organizacja, szkolenia i komunikacja

- [ ] Kuchnia i administratorzy przeszkoleni: zamówienia, QR, czytnik
      USB, raport, reset hasła, backup, plan papierowy.
- [ ] Wychowawcy przeszkoleni z przekazywania i niszczenia kart z
      hasłami; znają zakaz wysyłania CSV e-mailem, komunikatorem i do
      chmury.
- [ ] Uczniowie otrzymali instrukcję logowania i zasadę zmiany hasła
      przy pierwszym logowaniu.
- [ ] **Test z kilku telefonów** powtórzony na produkcji; znane są
      ograniczenia kamer (HTTPS/localhost) i działanie czytnika USB.
- [ ] Plan awaryjny kuchni (papier) jest wydrukowany, dostępny i
      przetestowany; wiadomo, kto go uruchamia.
- [ ] Dystrybucja haseł startowych: wydruk jednostronny w zamkniętym
      pomieszczeniu, natychmiastowe usunięcie CSV z komputera,
      folderu Pobrane i kosza, zniszczenie niewydanych kart.
- [ ] **Wyłącznik awaryjny** i **zgłaszanie naruszeń w 72 h przez
      szkołę** są opisane w widocznym miejscu i przetestowane.
- [ ] Monitoring i przegląd audytu mają właściciela (np. cotygodniowy
      przegląd zdarzeń bezpieczeństwa).
- [ ] Decyzja go/no-go produkcji podpisana przez dyrekcję, IOD i
      opiekuna technicznego.

## C. Przed włączeniem tunelu (Cloudflare)

> Kolejność jest krytyczna: **najpierw Cloudflare Access na panelach
> admina i kuchni, potem otwarcie tunelu**. Nigdy odwrotnie.

- [ ] Cloudflare Access jest skonfigurowany dla `/admin` **i** `/kitchen`
      (oraz świadomie dla `/student`) **przed** uruchomieniem tunelu.
- [ ] Zasady Access: domyślna odmowa, tylko zaufani użytkownicy/grupy
      szkoły, MFA, ograniczony czas sesji, osobna polityka dla admina.
- [ ] Sprawdzono z zewnątrz: bez logowania Cloudflare panel admina i
      kuchni **nie pokazują** formularza aplikacji, tylko ekran Access.
- [ ] Tunel działa jako `cloudflared` na tym samym hoście; aplikacja
      nasłuchuje na `127.0.0.1`, nie na publicznym interfejsie.
- [ ] Brak otwartego portu przychodzącego z internetu; firewall
      zweryfikowany skanem z zewnątrz.
- [ ] `PUBLIC_URL` ustawiony na docelowy `https://`, `COOKIE_SECURE=true`,
      HSTS i poprawne nagłówki bezpieczeństwa; brak mixed content.
- [ ] Origin guard i zaufane proxy są skonfigurowane świadomie; wiadomo,
      że limiter widzi adres proxy, jeśli nie ma obsługi zaufanego
      nagłówka.
- [ ] Sekrety tunelu i certyfikaty mają `0600`, nie są w repozytorium;
      jest plan ich rotacji i odbioru dostępu.
- [ ] Cloudflare Access loguje zdarzenia; ustalono retencję i dostęp do
      logów oraz umowę powierzenia przetwarzania (DPA), jeśli wymagana.
- [ ] WAF/rate limiting i ochrona przed botami są włączone świadomie,
      bez blokowania legalnego ruchu szkoły.
- [ ] Wykonano **skan OWASP ZAP przez tunel**, test z kilku telefonów i
      test kamery QR po HTTPS (jeśli używana).
- [ ] Plan awaryjny: wyłączenie Access, zatrzymanie tunelu, unieważnienie
      sesji, dezaktywacja kont, obrót `jwt.key` i powrót do trybu
      lokalnego są opisane i przetestowane.
- [ ] Zgłaszanie naruszeń przez szkołę (72 h) obejmuje dostawcę tunelu;
      znany jest zakres danych widocznych przez Cloudflare.
- [ ] Decyzja go/no-go dla tunelu podpisana przez opiekuna, IOD i
      dyrekcję.

## Rejestr odhaczeń

| Sekcja | Data | Właściciel | Wynik / dowód | Uwagi |
| --- | --- | --- | --- | --- |
| A. Pilot na Dellu | | | | |
| B. Produkcja | | | | |
| C. Tunel | | | | |

Po każdym incydencie lub istotnej zmianie (aktualizacja, nowy sprzęt,
zmiana sieci, włączenie tunelu) powtórz odpowiednie punkty i zapisz
nowy wpis w rejestrze.
