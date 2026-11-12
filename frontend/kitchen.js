/* ZSL Order — panel kuchni. */
(function () {
    "use strict";

    let ordersUi = null;
    let menuUi = null;
    let reportUi = null;
    let pickupContainer = null;
    let scannerStream = null;
    let scannerTimer = null;
    let audioContext = null;

    App.bootPage("kitchen", async function () {
        pickupContainer = App.$("#pickupResult");

        ordersUi = StaffUI.setupOrders({
            listSelector: "#ordersList",
            scopeSelector: ".seg[data-scope]",
            role: "kitchen",
            onLoaded: function (orders) {
                const open = orders.filter(function (order) {
                    return ["new", "preparing", "ready"].includes(order.status);
                }).length;
                App.setText(
                    "#ordersUpdated",
                    "Zaktualizowano " + new Date().toLocaleTimeString("pl-PL") +
                    " · otwarte: " + open
                );
            },
        });

        menuUi = StaffUI.setupMenuManager({
            listSelector: "#staffMenuList",
            dialogSelector: "#menuDialog",
            formSelector: "#menuForm",
            addButtonSelector: "#addMenuItemButton",
        });

        reportUi = StaffUI.setupDailyReport({
            dateSelector: "#reportDate",
            summarySelector: "#reportSummary",
            tableSelector: "#reportTableBody",
            updatedSelector: "#reportUpdated",
            exportButtonSelector: "#exportReportButton",
        });

        await ordersUi.refresh();
        await menuUi.refresh();

        App.$("#refreshOrdersButton").addEventListener("click", function () {
            ordersUi.refresh().then(function () {
                App.toast("Zamówienia odświeżone.", "success");
            }).catch(function (error) {
                App.toast(error.message, "error");
            });
        });

        App.$('.tab[data-tab="orders"]').addEventListener("click", function () {
            ordersUi.refresh().catch(function (error) { App.toast(error.message, "error"); });
        });
        App.$('.tab[data-tab="pickup"]').addEventListener("click", function () {
            focusPickupInput();
        });
        App.$('.tab[data-tab="menu"]').addEventListener("click", function () {
            menuUi.refresh().catch(function (error) { App.toast(error.message, "error"); });
        });
        App.$('.tab[data-tab="report"]').addEventListener("click", function () {
            reportUi.refresh().catch(function (error) { App.toast(error.message, "error"); });
        });
        App.$("#refreshReportButton").addEventListener("click", function () {
            reportUi.refresh().then(function () {
                App.toast("Raport odświeżony.", "success");
            }).catch(function (error) {
                App.toast(error.message, "error");
            });
        });

        const pickupForm = App.$("#pickupForm");
        pickupForm.addEventListener("submit", async function (event) {
            event.preventDefault();
            const code = App.$("#pickupCode").value.trim();
            if (!code) {
                App.toast("Wpisz kod odbioru.", "error");
                focusPickupInput();
                return;
            }
            await lookup(code);
        });

        setupScanner();

        // Odświeżanie otwartych zamówień, gdy karta jest widoczna.
        window.setInterval(function () {
            if (document.hidden) return;
            const ordersTab = App.$("#tab-orders");
            if (ordersTab && ordersTab.classList.contains("is-active")) {
                ordersUi.refresh().catch(function () {});
            }
        }, 8000);
    });

    function focusPickupInput() {
        const input = App.$("#pickupCode");
        if (!input) return;
        input.focus();
        input.select();
    }

    async function lookup(code) {
        const button = pickupFormButton();
        const input = App.$("#pickupCode");
        App.setBusy(button, true, "Sprawdzanie…");
        try {
            const data = await App.api.post("/api/staff/pickup", { code: code });
            renderPickup(data.order);
            feedback(true);
            input.value = "";
            focusPickupInput();
        } catch (error) {
            showPickupError(error.message);
            feedback(false);
            input.select();
            input.focus();
        } finally {
            App.setBusy(button, false);
        }
    }

    function pickupFormButton() {
        const form = App.$("#pickupForm");
        return form ? form.querySelector('button[type="submit"]') : null;
    }

    function renderPickup(order) {
        StaffUI.renderPickupResult(pickupContainer, order, async function () {
            App.clear(pickupContainer);
            App.$("#pickupCode").value = "";
            focusPickupInput();
            if (ordersUi) await ordersUi.refresh();
        });
    }

    function showPickupError(message) {
        App.clear(pickupContainer);
        pickupContainer.append(
            App.h("div", { class: "pickup-card pickup-error", attrs: { role: "alert" } }, [
                App.h("h3", { text: "Nie wydano zamówienia" }),
                App.h("p", { text: message }),
                App.h("p", {
                    class: "muted small",
                    text: "Popraw kod i spróbuj ponownie albo zeskanuj następny QR.",
                }),
            ])
        );
    }

    function feedback(ok) {
        try {
            const AudioCtor = window.AudioContext || window.webkitAudioContext;
            if (AudioCtor) {
                audioContext = audioContext || new AudioCtor();
                const oscillator = audioContext.createOscillator();
                const gain = audioContext.createGain();
                oscillator.type = "sine";
                oscillator.frequency.value = ok ? 880 : 220;
                gain.gain.value = 0.07;
                oscillator.connect(gain);
                gain.connect(audioContext.destination);
                oscillator.start();
                window.setTimeout(function () { oscillator.stop(); }, ok ? 120 : 320);
            }
        } catch (error) {
            // Dźwięk jest dodatkiem — brak AudioContext nie blokuje odbioru.
        }
        if (navigator.vibrate) {
            navigator.vibrate(ok ? 80 : [80, 60, 80]);
        }
    }

    function setupScanner() {
        const scanButton = App.$("#scanButton");
        const stopButton = App.$("#stopScanButton");
        const video = App.$("#scannerVideo");
        const status = App.$("#scannerStatus");

        const secure = window.isSecureContext;
        const hasMedia = !!(navigator.mediaDevices &&
            typeof navigator.mediaDevices.getUserMedia === "function");
        const hasDetector = "BarcodeDetector" in window;

        if (!secure) {
            status.textContent =
                "Kamera wymaga HTTPS lub połączenia z localhost. " +
                "W sieci lokalnej po HTTP użyj czytnika USB albo wpisz kod ręcznie. " +
                "Alternatywy: HTTPS w sieci lokalnej lub tryb tunnel.";
            return;
        }
        if (!hasMedia) {
            status.textContent =
                "Ta przeglądarka nie udostępnia kamery. Wpisz kod ręcznie albo użyj czytnika USB.";
            return;
        }
        if (!hasDetector) {
            status.textContent =
                "Ta przeglądarka nie obsługuje BarcodeDetector. " +
                "Wpisz kod ręcznie albo użyj czytnika USB (działa jak klawiatura).";
            return;
        }

        scanButton.hidden = false;

        scanButton.addEventListener("click", async function () {
            stopScanner();
            try {
                scannerStream = await navigator.mediaDevices.getUserMedia({
                    video: { facingMode: "environment" },
                    audio: false,
                });
                video.srcObject = scannerStream;
                video.hidden = false;
                stopButton.hidden = false;
                scanButton.hidden = true;
                status.textContent = "Skieruj kamerę na kod QR…";
                await video.play();

                const detector = new window.BarcodeDetector({ formats: ["qr_code"] });
                scannerTimer = window.setInterval(async function () {
                    if (!scannerStream) return;
                    try {
                        const codes = await detector.detect(video);
                        if (codes.length && codes[0].rawValue) {
                            const value = codes[0].rawValue;
                            stopScanner();
                            App.$("#pickupCode").value = value;
                            await lookup(value);
                        }
                    } catch (error) {
                        // Pomijamy pojedyncze klatki bez kodu.
                    }
                }, 350);
            } catch (error) {
                stopScanner();
                if (error && error.name === "NotAllowedError") {
                    status.textContent =
                        "Brak zgody na kamerę. Zezwól na dostęp w przeglądarce " +
                        "albo wpisz kod ręcznie / użyj czytnika USB.";
                } else {
                    status.textContent =
                        "Nie udało się uruchomić kamery. Wpisz kod ręcznie " +
                        "albo użyj czytnika USB.";
                }
                App.toast("Kamera wymaga HTTPS lub localhost.", "error");
                focusPickupInput();
            }
        });

        stopButton.addEventListener("click", function () {
            stopScanner();
            status.textContent = "Skanowanie zatrzymane.";
            focusPickupInput();
        });

        function stopScanner() {
            if (scannerTimer) {
                window.clearInterval(scannerTimer);
                scannerTimer = null;
            }
            if (scannerStream) {
                scannerStream.getTracks().forEach(function (track) { track.stop(); });
                scannerStream = null;
            }
            video.srcObject = null;
            video.hidden = true;
            stopButton.hidden = true;
            scanButton.hidden = false;
            status.textContent = "";
        }
    }
})();
