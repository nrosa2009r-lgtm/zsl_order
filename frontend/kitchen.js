/* ZSL Order — panel kuchni. */
(function () {
    "use strict";

    let ordersUi = null;
    let menuUi = null;
    let pickupContainer = null;
    let lastPickupCode = "";
    let scannerStream = null;
    let scannerTimer = null;

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
        App.$('.tab[data-tab="menu"]').addEventListener("click", function () {
            menuUi.refresh().catch(function (error) { App.toast(error.message, "error"); });
        });

        const pickupForm = App.$("#pickupForm");
        pickupForm.addEventListener("submit", async function (event) {
            event.preventDefault();
            const code = App.$("#pickupCode").value.trim();
            if (!code) {
                App.toast("Wpisz kod odbioru.", "error");
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

    async function lookup(code, silent) {
        const button = pickupFormButton();
        App.setBusy(button, true, "Sprawdzanie…");
        try {
            const data = await App.api.post("/api/staff/pickup", { code: code });
            lastPickupCode = code;
            renderPickup(data.order);
            if (!silent) App.$("#pickupCode").value = "";
        } catch (error) {
            if (!silent) {
                App.clear(pickupContainer);
                App.toast(error.message, "error");
            }
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
            if (lastPickupCode) await lookup(lastPickupCode, true);
            if (ordersUi) await ordersUi.refresh();
        });
    }

    function setupScanner() {
        const scanButton = App.$("#scanButton");
        const stopButton = App.$("#stopScanButton");
        const video = App.$("#scannerVideo");
        const status = App.$("#scannerStatus");

        const supported = ("BarcodeDetector" in window)
            && navigator.mediaDevices
            && typeof navigator.mediaDevices.getUserMedia === "function";

        if (!supported) {
            status.textContent = "Skaner kamery nie jest dostępny w tej przeglądarce. Użyj kodu lub czytnika USB.";
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
                status.textContent = "Nie udało się uruchomić kamery. Użyj kodu lub czytnika USB.";
                App.toast("Kamera wymaga HTTPS lub połączenia z localhost.", "error");
            }
        });

        stopButton.addEventListener("click", function () {
            stopScanner();
            status.textContent = "Skanowanie zatrzymane.";
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
