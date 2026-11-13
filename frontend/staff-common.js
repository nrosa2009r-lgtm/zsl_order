/* SPDX-License-Identifier: MIT */
/* ZSL Order — wspólne elementy panelu kuchni i administratora. */
(function () {
    "use strict";

    function setupMenuManager(options) {
        const list = App.$(options.listSelector);
        const dialog = App.$(options.dialogSelector);
        const form = App.$(options.formSelector);
        const addButton = App.$(options.addButtonSelector);
        let items = [];

        function openCreate() {
            App.$("#menuItemId").value = "";
            App.$("#menuDialogTitle").textContent = "Dodaj pozycję";
            App.$("#menuName").value = "";
            App.$("#menuDescription").value = "";
            App.$("#menuPrice").value = "";
            App.$("#menuCategory").value = "Dania";
            App.$("#menuAllergens").value = "";
            App.$("#menuSortOrder").value = "0";
            App.$("#menuAvailable").checked = true;
            App.$("#menuFormError").hidden = true;
            dialog.showModal();
        }

        function openEdit(item) {
            App.$("#menuItemId").value = String(item.id);
            App.$("#menuDialogTitle").textContent = "Edytuj pozycję";
            App.$("#menuName").value = item.name;
            App.$("#menuDescription").value = item.description || "";
            App.$("#menuPrice").value = (item.price_cents / 100).toFixed(2);
            App.$("#menuCategory").value = item.category || "Dania";
            App.$("#menuAllergens").value = item.allergens || "";
            App.$("#menuSortOrder").value = String(item.sort_order || 0);
            App.$("#menuAvailable").checked = !!item.available;
            App.$("#menuFormError").hidden = true;
            dialog.showModal();
        }

        async function refresh() {
            items = await App.api.get("/api/staff/menu");
            App.clear(list);

            if (!items.length) {
                list.append(App.h("p", { class: "empty-state", text: "Menu jest puste. Dodaj pierwszą pozycję." }));
                return;
            }

            items.forEach(function (item) {
                const card = App.h("article", { class: "manage-item" });
                const main = App.h("div", { class: "manage-main" });
                main.append(
                    App.h("div", { class: "manage-title" }, [
                        App.h("h3", { text: item.name }),
                        item.available
                            ? App.h("span", { class: "badge badge-ready", text: "Dostępna" })
                            : App.h("span", { class: "badge badge-cancelled", text: "Wycofana" }),
                    ]),
                    App.h("p", { class: "muted small", text: item.description || "Brak opisu." })
                );
                if (item.allergens) {
                    main.append(App.h("p", { class: "allergens", text: "Alergeny: " + item.allergens }));
                }

                const meta = App.h("div", { class: "manage-meta" });
                meta.append(
                    App.h("span", { class: "chip", text: item.category || "Dania" }),
                    App.h("strong", { class: "price", text: App.fmtPLN(item.price_cents) })
                );

                const actions = App.h("div", { class: "manage-actions" });
                actions.append(
                    App.h("button", {
                        class: "btn btn-ghost btn-small",
                        text: "Edytuj",
                        attrs: { type: "button" },
                        on: { click: function () { openEdit(item); } },
                    }),
                    App.h("button", {
                        class: "btn btn-secondary btn-small",
                        text: item.available ? "Wycofaj" : "Przywróć",
                        attrs: { type: "button" },
                        on: {
                            click: async function (event) {
                                const button = event.currentTarget;
                                App.setBusy(button, true, "Zapisywanie…");
                                try {
                                    await App.api.put("/api/staff/menu/" + item.id, Object.assign({}, item, {
                                        available: !item.available,
                                    }));
                                    App.toast(item.available ? "Pozycja wycofana." : "Pozycja przywrócona.", "success");
                                    await refresh();
                                    if (options.onChanged) options.onChanged();
                                } catch (error) {
                                    App.toast(error.message, "error");
                                    App.setBusy(button, false);
                                }
                            },
                        },
                    }),
                    App.h("button", {
                        class: "btn btn-danger btn-small",
                        text: "Usuń",
                        attrs: { type: "button" },
                        on: {
                            click: async function () {
                                if (!window.confirm("Usunąć pozycję z menu? Historia zamówień pozostanie.")) return;
                                try {
                                    await App.api.del("/api/staff/menu/" + item.id);
                                    App.toast("Pozycja usunięta.", "success");
                                    await refresh();
                                    if (options.onChanged) options.onChanged();
                                } catch (error) {
                                    App.toast(error.message, "error");
                                }
                            },
                        },
                    })
                );

                card.append(main, meta, actions);
                list.append(card);
            });
        }

        if (addButton) addButton.addEventListener("click", openCreate);

        form.addEventListener("submit", async function (event) {
            event.preventDefault();
            const errorNode = App.$("#menuFormError");
            const saveButton = App.$("#menuSaveButton");
            errorNode.hidden = true;

            const priceValue = Number.parseFloat(App.$("#menuPrice").value.replace(",", "."));
            if (!Number.isFinite(priceValue) || priceValue < 0 || priceValue > 1000) {
                errorNode.textContent = "Podaj poprawną cenę od 0 do 1000 zł.";
                errorNode.hidden = false;
                return;
            }

            const payload = {
                name: App.$("#menuName").value.trim(),
                description: App.$("#menuDescription").value.trim(),
                price_cents: Math.round(priceValue * 100),
                category: App.$("#menuCategory").value.trim(),
                allergens: App.$("#menuAllergens").value.trim(),
                available: App.$("#menuAvailable").checked,
                sort_order: Number.parseInt(App.$("#menuSortOrder").value, 10) || 0,
            };

            if (!payload.name) {
                errorNode.textContent = "Nazwa jest wymagana.";
                errorNode.hidden = false;
                return;
            }

            const id = App.$("#menuItemId").value;
            App.setBusy(saveButton, true, "Zapisywanie…");
            try {
                if (id) {
                    await App.api.put("/api/staff/menu/" + id, payload);
                    App.toast("Pozycja zaktualizowana.", "success");
                } else {
                    await App.api.post("/api/staff/menu", payload);
                    App.toast("Pozycja dodana.", "success");
                }
                dialog.close();
                await refresh();
                if (options.onChanged) options.onChanged();
            } catch (error) {
                errorNode.textContent = error.message;
                errorNode.hidden = false;
            } finally {
                App.setBusy(saveButton, false, "Zapisz");
            }
        });

        return { refresh: refresh };
    }

    async function updateOrder(id, payload, options) {
        try {
            await App.api.patch("/api/staff/orders/" + id, payload);
            App.toast("Zamówienie zaktualizowane.", "success");
            if (options && options.onUpdated) await options.onUpdated();
        } catch (error) {
            App.toast(error.message, "error");
        }
    }

    function orderCard(order, options) {
        options = options || {};
        const role = options.role || "kitchen";
        const card = App.h("article", {
            class: "order-card order-" + order.status,
        });

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
        const metaPairs = [
            ["Uczeń (kod)", order.student_code || "—"],
            ["Złożono", App.fmtDate(order.created_at)],
            ["Odbiór", order.pickup_time || "Bez godziny"],
            ["Płatność", App.paymentLabels[order.payment_method] || order.payment_method],
            ["Kwota", App.fmtPLN(order.total_cents)],
        ];
        if (order.paid_at) {
            metaPairs.push(["Opłacono", App.fmtDate(order.paid_at)]);
        }
        if (order.paid_by) {
            metaPairs.push(["Opłacone przez", order.paid_by]);
        }
        metaPairs.forEach(function (pair) {
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

        const detail = App.h("div", { class: "order-detail" });
        detail.append(meta, items);

        const actions = App.h("div", { class: "order-actions" });

        if (order.status === "uncollected") {
            actions.append(App.h("span", {
                class: "muted small",
                text: role === "admin"
                    ? "Zamówienie nieodebrane. Możesz je skorygować."
                    : "Zamówienie nieodebrane — zamknięte.",
            }));
        }

        if (role === "kitchen") {
            if (order.status === "new") {
                actions.append(App.h("button", {
                    class: "btn btn-primary btn-small",
                    text: "Przyjmij do realizacji",
                    attrs: { type: "button" },
                    on: {
                        click: function () {
                            updateOrder(order.id, { status: "preparing" }, options);
                        },
                    },
                }));
            } else if (order.status === "preparing") {
                actions.append(App.h("button", {
                    class: "btn btn-primary btn-small",
                    text: "Gotowe do odbioru",
                    attrs: { type: "button" },
                    on: {
                        click: function () {
                            updateOrder(order.id, { status: "ready" }, options);
                        },
                    },
                }));
            } else if (order.status === "ready") {
                actions.append(App.h("button", {
                    class: "btn btn-success btn-small",
                    text: "Wydano i opłacone",
                    attrs: { type: "button" },
                    on: {
                        click: function () {
                            updateOrder(order.id, { status: "completed" }, options);
                        },
                    },
                }));
            }

            if (order.payment_status === "unpaid" && order.status === "ready") {
                actions.append(App.h("button", {
                    class: "btn btn-secondary btn-small",
                    text: "Oznacz jako opłacone",
                    attrs: { type: "button" },
                    on: {
                        click: function () {
                            updateOrder(order.id, { payment_status: "paid" }, options);
                        },
                    },
                }));
            }
        } else if (order.status === "uncollected") {
            actions.append(App.h("button", {
                class: "btn btn-secondary btn-small",
                text: "Skoryguj: wydane",
                attrs: { type: "button" },
                on: {
                    click: function () {
                        if (!window.confirm("Przywrócić zamówienie jako wydane i zdjąć znacznik nieodebranego?")) return;
                        updateOrder(order.id, { status: "completed" }, options);
                    },
                },
            }));
        } else {
            const statusSelect = App.h("select", {
                attrs: { "aria-label": "Status zamówienia" },
            });
            [
                ["new", "Nowe"],
                ["preparing", "W przygotowaniu"],
                ["ready", "Gotowe"],
                ["completed", "Wydane"],
                ["cancelled", "Anulowane"],
            ].forEach(function (pair) {
                const option = App.h("option", { text: pair[1], attrs: { value: pair[0] } });
                if (pair[0] === order.status) option.selected = true;
                statusSelect.append(option);
            });

            const paymentSelect = App.h("select", {
                attrs: { "aria-label": "Status płatności" },
            });
            [
                ["unpaid", "Nieopłacone"],
                ["paid", "Opłacone"],
            ].forEach(function (pair) {
                const option = App.h("option", { text: pair[1], attrs: { value: pair[0] } });
                if (pair[0] === order.payment_status) option.selected = true;
                paymentSelect.append(option);
            });

            const save = App.h("button", {
                class: "btn btn-primary btn-small",
                text: "Zapisz",
                attrs: { type: "button" },
                on: {
                    click: function () {
                        updateOrder(order.id, {
                            status: statusSelect.value,
                            payment_status: paymentSelect.value,
                        }, options);
                    },
                },
            });

            actions.append(statusSelect, paymentSelect, save);

            if (order.status !== "cancelled") {
                actions.append(App.h("button", {
                    class: "btn btn-danger btn-small",
                    text: "Anuluj",
                    attrs: { type: "button" },
                    on: {
                        click: function () {
                            if (!window.confirm("Anulować zamówienie?")) return;
                            updateOrder(order.id, { status: "cancelled" }, options);
                        },
                    },
                }));
            }
        }

        const footer = App.h("div", { class: "order-footer" });
        footer.append(
            App.h("strong", { text: "Razem: " + App.fmtPLN(order.total_cents) }),
            actions
        );

        card.append(head, detail, footer);
        return card;
    }

    function setupOrders(options) {
        const list = App.$(options.listSelector);
        const scopeButtons = App.$$(options.scopeSelector);
        let scope = options.initialScope || "open";
        let orders = [];

        async function refresh() {
            orders = await App.api.get("/api/staff/orders?scope=" + encodeURIComponent(scope));
            render();
            if (options.onLoaded) options.onLoaded(orders);
        }

        function render() {
            App.clear(list);
            if (!orders.length) {
                list.append(
                    App.h("p", {
                        class: "empty-state",
                        text: scope === "open" ? "Brak otwartych zamówień." : "Brak zamówień w tym widoku.",
                    })
                );
                return;
            }

            orders.forEach(function (order) {
                list.append(orderCard(order, {
                    role: options.role,
                    onUpdated: async function () {
                        await refresh();
                        if (options.onUpdated) options.onUpdated();
                    },
                }));
            });
        }

        scopeButtons.forEach(function (button) {
            button.addEventListener("click", function () {
                scope = button.getAttribute("data-scope") || "open";
                scopeButtons.forEach(function (other) {
                    other.classList.toggle("is-active", other === button);
                });
                refresh().catch(function (error) { App.toast(error.message, "error"); });
            });
        });

        return {
            refresh: refresh,
            getScope: function () { return scope; },
        };
    }

    function localToday() {
        // Doba raportu jest zawsze warszawska, niezależnie od ustawień
        // przeglądarki (np. laptop ucznia na wyjeździe).
        return new Intl.DateTimeFormat("en-CA", {
            timeZone: "Europe/Warsaw",
            year: "numeric",
            month: "2-digit",
            day: "2-digit",
        }).format(new Date());
    }

    function setupDailyReport(options) {
        const dateInput = App.$(options.dateSelector);
        const summary = App.$(options.summarySelector);
        const tbody = App.$(options.tableSelector);
        const updated = App.$(options.updatedSelector);
        const exportButton = App.$(options.exportButtonSelector);

        if (dateInput && !dateInput.value) dateInput.value = localToday();

        function stat(label, value, hint) {
            return App.h("div", { class: "stat-card" }, [
                App.h("span", { class: "stat-label", text: label }),
                App.h("strong", { class: "stat-value", text: value }),
                App.h("span", { class: "stat-hint", text: hint }),
            ]);
        }

        function renderRow(label, totals) {
            const row = App.h("tr");
            [
                label,
                totals.orders,
                totals.paid_orders,
                App.fmtPLN(totals.paid_cents),
                totals.unpaid_orders,
                App.fmtPLN(totals.unpaid_cents),
                totals.cancelled_orders,
                totals.uncollected_orders,
            ].forEach(function (value) {
                row.append(App.h("td", { text: value }));
            });
            tbody.append(row);
        }

        async function refresh() {
            const date = (dateInput && dateInput.value) || localToday();
            const report = await App.api.get("/api/staff/report?date=" + encodeURIComponent(date));

            App.clear(summary);
            summary.append(
                stat("Zamówienia", report.orders, "złożone w dniu"),
                stat("Opłacone", report.paid_orders, App.fmtPLN(report.paid_cents) + " (z nieodebranymi)"),
                stat("Nieopłacone", report.unpaid_orders, App.fmtPLN(report.unpaid_cents)),
                stat("Nieodebrane", report.uncollected_orders, "poza opłaconymi"),
                stat("Anulowane", report.cancelled_orders, "poza nieodebranymi")
            );

            App.clear(tbody);
            renderRow("Gotówka", report.cash);
            renderRow("Karta", report.card);
            renderRow("Razem", report);

            if (updated) {
                updated.textContent = "Raport za " + report.date +
                    " · zaktualizowano " + new Date().toLocaleTimeString("pl-PL");
            }
        }

        if (dateInput) {
            dateInput.addEventListener("change", function () {
                refresh().catch(function (error) { App.toast(error.message, "error"); });
            });
        }
        if (exportButton) {
            exportButton.addEventListener("click", function () {
                const date = (dateInput && dateInput.value) || localToday();
                window.location.href = "/api/staff/report.csv?date=" + encodeURIComponent(date);
            });
        }

        return { refresh: refresh };
    }

    function renderPickupResult(container, order, onIssued) {
        App.clear(container);

        const card = App.h("article", { class: "pickup-card pickup-" + order.status });
        const head = App.h("div", { class: "pickup-head" });
        head.append(
            App.h("div", {}, [
                App.h("p", { class: "muted tiny", text: "Znaleziono zamówienie" }),
                App.h("h3", { text: order.code }),
            ]),
            App.h("div", { class: "badge-row" }, [
                App.statusBadge(order.status),
                App.paymentBadge(order.payment_status),
            ])
        );

        const meta = App.h("dl", { class: "pickup-meta" });
        [
            ["Uczeń (kod)", order.student_code || "—"],
            ["Złożono", App.fmtDate(order.created_at)],
            ["Odbiór", order.pickup_time || "Bez godziny"],
            ["Płatność", App.paymentLabels[order.payment_method] || order.payment_method],
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

        const total = App.h("div", { class: "pickup-total" }, [
            App.h("span", {
                text: order.payment_status === "unpaid" ? "Do zapłaty" : "Opłacone",
            }),
            App.h("strong", { text: App.fmtPLN(order.total_cents) }),
        ]);

        const note = App.h("p", {
            class: "pickup-note",
            text: order.payment_status === "unpaid"
                ? "Przy wydaniu płatność zostanie automatycznie oznaczona jako opłacona."
                : "Zamówienie jest już opłacone.",
        });

        const issue = App.h("button", {
            class: "btn btn-success btn-large pickup-issue",
            text: "Wydaj",
            attrs: { type: "button" },
            on: {
                click: async function () {
                    App.setBusy(issue, true, "Wydawanie…");
                    try {
                        await App.api.patch("/api/staff/orders/" + order.id, {
                            status: "completed",
                        });
                        App.toast("Zamówienie wydane.", "success");
                        if (onIssued) await onIssued();
                    } catch (error) {
                        App.toast(error.message, "error");
                        App.setBusy(issue, false, "Wydaj");
                    }
                },
            },
        });

        card.append(head, meta, items, total, note, issue);
        container.append(card);
    }

    window.StaffUI = {
        setupMenuManager: setupMenuManager,
        setupOrders: setupOrders,
        orderCard: orderCard,
        setupDailyReport: setupDailyReport,
        renderPickupResult: renderPickupResult,
    };
})();
