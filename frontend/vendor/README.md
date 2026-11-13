# Zależności frontendu (vendored)

Pliki w tym katalogu są serwowane wyłącznie z własnego serwera. Aplikacja
nie używa CDN, a CSP pozostaje `script-src 'self'`.

## jsQR 1.4.0

- Projekt: <https://github.com/cozmo/jsQR>
- Źródło pobrania: oficjalny pakiet npm `jsqr@1.4.0`
  (`https://registry.npmjs.org/jsqr/-/jsqr-1.4.0.tgz`), opublikowany
  z repozytorium `cozmo/jsQR`.
- SHA-256 archiwum npm:
  `b5299b37917a1fe7a8cab9dd5cc6b8accf82663add80abe5bf7761a921cc6602`
- SHA-256 oryginalnego `dist/jsQR.js`:
  `bc40c8a15196236b2314db0856f72ca0b49980cd5413b8c852a7349f5fee0859`
- SHA-256 pliku w repozytorium (`jsQR.js`):
  `9468f640af6a23b0046dab2f0738cd41c95cd86701907f533f7cd92654039464`
- Różnica względem oryginału: na początku pliku dodano wyłącznie
  jednolinijkowy nagłówek z wersją, autorem i licencją. Kod biblioteki
  nie został zmieniony.
- Licencja: Apache-2.0. Pełny tekst w [`jsQR.LICENSE`](jsQR.LICENSE);
  wpis w rejestrze zależności: [`../../THIRD-PARTY-LICENSES.md`](../../THIRD-PARTY-LICENSES.md).

### Sposób użycia

`kitchen.js` ładuje `/vendor/jsQR.js` dynamicznie **tylko na stronie
kuchni** i **tylko wtedy**, gdy przeglądarka nie udostępnia
`BarcodeDetector`. Plik nie jest dołączany statycznie do HTML. Kamera
nadal wymaga bezpiecznego kontekstu (HTTPS albo `localhost`); jsQR
zastępuje wyłącznie dekoder, nie omija ograniczeń `getUserMedia`.

### Aktualizacja

1. Pobierz nową wersję z oficjalnego źródła.
2. Zapisz nowe sumy SHA-256 powyżej.
3. Dodaj lub zachowaj nagłówek licencji.
4. Uruchom `scripts/smoke-hardening.sh` — test sprawdza, że plik jest
   serwowany z poprawnym `Content-Type` i bez CDN.
