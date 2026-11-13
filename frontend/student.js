/* SPDX-License-Identifier: MIT */
/* ZSL Order — panel ucznia. */
(function () {
    "use strict";

    let menu = [];
    let cart = new Map();
    let info = null;
    let checkoutKey = null;

    function newIdempotencyKey() {
        if (window.crypto && typeof window.crypto.randomUUID === "function") {
            return window.crypto.randomUUID();
        }
        return "k-" + Date.now().toString(36) + "-" +
            Math.random().toString(36).slice(2, 12) + "-" +
            Math.random().toString(36).slice(2, 12);
    }

    function switchTab(name) {
        const tab = App.$('.tab[data-tab="' + name + '"]');
        if (tab) tab.click();
    }

    async function loadMenu() {
        const data = await App.api.get("/api/student/menu");
        menu = data.items || [];
        info = data;

        const enabled = !!data.ordering_enabled;
        const banner = App.$("#orderingBanner");
        if (banner) {
            banner.hidden = enabled;
            banner.textContent = enabled
                ? ""
                : "Zamawianie jest chwilowo wyłączone. Menu możesz przejrzeć, ale nie złożysz zamówienia.";
        }
        App.setText(
            "#orderingStatus",
            enabled
                ? (data.payment_info || "Wybierz pozycje i złóż zamówienie. Płatność przy odbiorze.")
                : "Zamawianie wyłączone."
        );
        const checkoutButton = App.$("#checkoutButton");
        if (checkoutButton) checkoutButton.disabled = !enabled;
        renderMenu();
        renderCart();
    }

    function renderMenu() {
        const list = App.$("#menuList");
        App.clear(list);

        if (!menu.length) {
            list.append(
                App.h("p", {
                    class: "empty-state",
                    text: "Menu jest jeszcze puste. Sprawdź ponownie później.",
                })
            );
            return;
        }

        menu.forEach(function (item) {
            const card = App.h("article", { class: "menu-card" });
            const top = App.h("div", { class: "menu-card-top" });
            top.append(
                App.h("span", { class: "chip", text: item.category || "Dania" }),
                App.h("strong", { class: "price", text: App.fmtPLN(item.price_cents) })
            );

            card.append(
                top,
                App.h("h3", { text: item.name }),
                App.h("p", {
                    class: "muted small",
                    text: item.description || "Brak opisu.",
                })
            );

            if (item.allergens) {
                card.append(
                    App.h("p", {
                        class: "allergens",
                        text: "Alergeny: " + item.allergens,
                    })
                );
            }

            const inCart = cart.get(item.id) || 0;
            const addButton = App.h("button", {
                class: "btn btn-secondary btn-block",
                text: inCart ? "Dodaj kolejną (" + inCart + ")" : "Dodaj do zamówienia",
                attrs: { type: "button" },
                on: {
                    click: function () { addToCart(item.id); },
                },
            });
            if (!info || !info.ordering_enabled) addButton.disabled = true;
            card.append(addButton);
            list.append(card);
        });
    }

    function addToCart(id) {
        const current = cart.get(id) || 0;
        const maxItems = info && info.max_order_items ? info.max_order_items : 20;
        if (current >= 10) {
            App.toast("Maksymalnie 10 sztuk jednej pozycji.", "error");
            return;
        }
        if (current === 0 && cart.size >= maxItems) {
            App.toast("Zamówienie może zawierać maksymalnie " + maxItems + " różnych pozycji.", "error");
            return;
        }
        cart.set(id, current + 1);
        renderMenu();
        renderCart();
    }

    function changeQuantity(id, delta) {
        const current = cart.get(id) || 0;
        const next = current + delta;
        if (next <= 0) {
            cart.delete(id);
        } else if (next > 10) {
            App.toast("Maksymalnie 10 sztuk jednej pozycji.", "error");
            return;
        } else {
            cart.set(id, next);
        }
        renderMenu();
        renderCart();
    }

    function cartEntries() {
        const result = [];
        cart.forEach(function (quantity, id) {
            const item = menu.find(function (entry) { return entry.id === id; });
            if (item) result.push({ item: item, quantity: quantity });
        });
        return result;
    }

    function renderCart() {
        const container = App.$("#cartItems");
        const entries = cartEntries();
        App.clear(container);

        if (!entries.length) {
            container.append(
                App.h("p", { class: "muted small", text: "Koszyk jest pusty. Dodaj pozycje z menu." })
            );
        } else {
            entries.forEach(function (entry) {
                const row = App.h("div", { class: "cart-row" });
                const infoBox = App.h("div", { class: "cart-row-info" });
                infoBox.append(
                    App.h("span", { text: entry.item.name }),
                    App.h("span", {
                        class: "muted small",
                        text: App.fmtPLN(entry.item.price_cents * entry.quantity),
                    })
                );

                const controls = App.h("div", { class: "qty-controls" });
                const minus = App.h("button", {
                    class: "qty-btn",
                    text: "−",
                    attrs: { type: "button", "aria-label": "Zmniejsz ilość" },
                    on: { click: function () { changeQuantity(entry.item.id, -1); } },
                });
                const plus = App.h("button", {
                    class: "qty-btn",
                    text: "+",
                    attrs: { type: "button", "aria-label": "Zwiększ ilość" },
                    on: { click: function () { changeQuantity(entry.item.id, 1); } },
                });
                controls.append(minus, App.h("span", { class: "qty-value", text: entry.quantity }), plus);
                row.append(infoBox, controls);
                container.append(row);
            });
        }

        const total = entries.reduce(function (sum, entry) {
            return sum + entry.item.price_cents * entry.quantity;
        }, 0);
        App.setText("#cartTotal", App.fmtPLN(total));
        App.setText("#cartCount", String(entries.reduce(function (sum, entry) {
            return sum + entry.quantity;
        }, 0)));

        const checkout = App.$("#checkoutButton");
        if (checkout) checkout.disabled = !info || !info.ordering_enabled;
    }

    async function checkout(event) {
        event.preventDefault();
        const errorNode = App.$("#cartError");
        const button = App.$("#checkoutButton");
        errorNode.hidden = true;

        const entries = cartEntries();
        if (!entries.length) {
            errorNode.textContent = "Dodaj co najmniej jedną pozycję do zamówienia.";
            errorNode.hidden = false;
            return;
        }

        const paymentInput = App.$('input[name="payment_method"]:checked');
        const paymentMethod = paymentInput ? paymentInput.value : "cash";
        const pickupTime = App.$("#pickupTime").value || "";

        // Ten sam klucz przy ponowieniu po timeoucie zwraca istniejące
        // zamówienie zamiast tworzyć drugie.
        if (!checkoutKey) checkoutKey = newIdempotencyKey();
        const payload = {
            payment_method: paymentMethod,
            pickup_time: pickupTime,
            idempotency_key: checkoutKey,
            items: entries.map(function (entry) {
                return { menu_item_id: entry.item.id, quantity: entry.quantity };
            }),
        };

        App.setBusy(button, true, "Składanie…");
        try {
            const order = await App.api.post("/api/student/orders", payload);
            checkoutKey = null;
            cart.clear();
            renderMenu();
            renderCart();
            App.$("#pickupTime").value = "";
            showOrderConfirmation(order);
            await loadOrders();
            switchTab("orders");
        } catch (error) {
            errorNode.textContent = error.message;
            errorNode.hidden = false;
        } finally {
            App.setBusy(button, false, "Złóż zamówienie");
        }
    }

    function showOrderConfirmation(order) {
        const dialog = App.h("dialog", { class: "dialog order-confirm" });
        const title = App.h("h2", { text: "Zamówienie przyjęte" });
        const subtitle = App.h("p", {
            class: "muted",
            text: "Pokaż ten kod lub QR przy ladzie. Płatność przy odbiorze.",
        });

        const codeBox = App.h("div", { class: "pickup-code" });
        codeBox.append(
            App.h("span", { class: "muted tiny", text: "Kod odbioru" }),
            App.h("strong", { text: order.code })
        );

        const qr = App.h("img", {
            class: "qr-image",
            attrs: {
                src: "/api/student/orders/" + order.id + "/qr.svg",
                alt: "Kod QR do odbioru zamówienia " + order.code,
                width: "280",
                height: "280",
            },
        });

        const total = App.h("p", {
            class: "order-total",
            text: "Razem: " + App.fmtPLN(order.total_cents),
        });

        const close = App.h("button", {
            class: "btn btn-primary btn-block",
            text: "Gotowe",
            attrs: { type: "button" },
            on: {
                click: function () { dialog.close(); },
            },
        });

        dialog.append(title, subtitle, codeBox, qr, total, close);
        dialog.addEventListener("close", function () { dialog.remove(); });
        document.body.append(dialog);
        if (typeof dialog.showModal === "function") dialog.showModal();
    }

    async function loadOrders() {
        const orders = await App.api.get("/api/student/orders");
        const list = App.$("#ordersList");
        App.clear(list);

        if (!orders.length) {
            list.append(
                App.h("p", { class: "empty-state", text: "Nie masz jeszcze zamówień." })
            );
            return;
        }

        orders.forEach(function (order) {
            list.append(renderOrder(order));
        });
    }

    function renderOrder(order) {
        const card = App.h("article", { class: "order-card" });
        const head = App.h("div", { class: "order-head" });
        const code = App.h("div", { class: "order-code" });
        code.append(
            App.h("span", { class: "muted tiny", text: "Kod odbioru" }),
            App.h("strong", { text: order.code })
        );
        const badges = App.h("div", { class: "badge-row" }, [
            App.statusBadge(order.status),
            App.paymentBadge(order.payment_status),
        ]);
        head.append(code, badges);

        const meta = App.h("dl", { class: "order-meta" });
        [
            ["Złożono", App.fmtDate(order.created_at)],
            ["Odbiór", order.pickup_time || "Bez godziny"],
            ["Płatność", App.paymentLabels[order.payment_method] || order.payment_method],
            ["Opłacono", order.paid_at ? App.fmtDate(order.paid_at) : "—"],
        ].forEach(function (pair) {
            const wrapper = App.h("div");
            wrapper.append(App.h("dt", { text: pair[0] }), App.h("dd", { text: pair[1] }));
            meta.append(wrapper);
        });

        const items = App.h("ul", { class: "order-items" });
        order.items.forEach(function (item) {
            items.append(
                App.h("li", {}, [
                    App.h("span", { text: item.quantity + " × " + item.name }),
                    App.h("span", { class: "muted", text: App.fmtPLN(item.price_cents * item.quantity) }),
                ])
            );
        });

        const body = App.h("div", { class: "order-body" });
        const detail = App.h("div", { class: "order-detail" });
        detail.append(meta, items);
        body.append(detail);

        if (["new", "preparing", "ready"].includes(order.status)) {
            const qrBox = App.h("div", { class: "order-qr" });
            qrBox.append(
                App.h("img", {
                    class: "qr-image small",
                    attrs: {
                        src: "/api/student/orders/" + order.id + "/qr.svg",
                        alt: "Kod QR zamówienia " + order.code,
                        loading: "lazy",
                        width: "180",
                        height: "180",
                    },
                })
            );
            body.append(qrBox);
        }

        const footer = App.h("div", { class: "order-footer" });
        footer.append(
            App.h("strong", { text: "Razem: " + App.fmtPLN(order.total_cents) })
        );

        if (order.status === "new" && order.payment_status !== "paid") {
            const cancel = App.h("button", {
                class: "btn btn-danger btn-small",
                text: "Anuluj zamówienie",
                attrs: { type: "button" },
                on: {
                    click: async function () {
                        if (!window.confirm("Czy na pewno anulować to zamówienie?")) return;
                        try {
                            await App.api.post("/api/student/orders/" + order.id + "/cancel", {});
                            App.toast("Zamówienie anulowane.", "success");
                            await loadOrders();
                        } catch (error) {
                            App.toast(error.message, "error");
                        }
                    },
                },
            });
            footer.append(cancel);
        }

        card.append(head, body, footer);
        return card;
    }

    App.bootPage("student", async function () {
        await loadMenu();
        await loadOrders();

        App.$("#refreshMenuButton").addEventListener("click", async function () {
            try {
                await loadMenu();
                App.toast("Menu odświeżone.", "success");
            } catch (error) {
                App.toast(error.message, "error");
            }
        });

        App.$("#refreshOrdersButton").addEventListener("click", async function () {
            try {
                await loadOrders();
                App.toast("Zamówienia odświeżone.", "success");
            } catch (error) {
                App.toast(error.message, "error");
            }
        });

        App.$("#checkoutForm").addEventListener("submit", checkout);
        App.$('.tab[data-tab="orders"]').addEventListener("click", function () {
            loadOrders().catch(function (error) { App.toast(error.message, "error"); });
        });
        App.$('.tab[data-tab="menu"]').addEventListener("click", function () {
            loadMenu().catch(function (error) { App.toast(error.message, "error"); });
        });
    });
})();
