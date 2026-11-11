/* ZSL Order — panel administratora. */
(function () {
    "use strict";

    let ordersUi = null;
    let menuUi = null;
    let reportUi = null;
    let generatedAccounts = [];
    let activityData = null;
    let resizeTimer = null;

    App.bootPage("admin", async function () {
        menuUi = StaffUI.setupMenuManager({
            listSelector: "#staffMenuList",
            dialogSelector: "#menuDialog",
            formSelector: "#menuForm",
            addButtonSelector: "#addMenuItemButton",
        });

        ordersUi = StaffUI.setupOrders({
            listSelector: "#ordersList",
            scopeSelector: ".seg[data-scope]",
            role: "admin",
        });

        reportUi = StaffUI.setupDailyReport({
            dateSelector: "#reportDate",
            summarySelector: "#reportSummary",
            tableSelector: "#reportTableBody",
            updatedSelector: "#reportUpdated",
            exportButtonSelector: "#exportReportButton",
        });

        await loadStats();
        await loadActivity();
        await loadUsers();
        await loadSettings();
        await menuUi.refresh();
        await ordersUi.refresh();
        await loadAudit();

        App.$("#refreshStatsButton").addEventListener("click", function () {
            loadStats().then(function () { App.toast("Statystyki odświeżone.", "success"); })
                .catch(function (error) { App.toast(error.message, "error"); });
        });
        App.$("#refreshActivityButton").addEventListener("click", function () {
            loadActivity().then(function () { App.toast("Wykresy odświeżone.", "success"); })
                .catch(function (error) { App.toast(error.message, "error"); });
        });
        App.$("#activityDays").addEventListener("change", function () {
            loadActivity().catch(function (error) { App.toast(error.message, "error"); });
        });
        App.$("#exportUsersButton").addEventListener("click", exportUsersCsv);
        App.$("#refreshOrdersButton").addEventListener("click", function () {
            ordersUi.refresh().then(function () { App.toast("Zamówienia odświeżone.", "success"); })
                .catch(function (error) { App.toast(error.message, "error"); });
        });
        App.$("#refreshAuditButton").addEventListener("click", function () {
            loadAudit().then(function () { App.toast("Dziennik odświeżony.", "success"); })
                .catch(function (error) { App.toast(error.message, "error"); });
        });
        App.$("#filterUsersButton").addEventListener("click", function () {
            loadUsers().catch(function (error) { App.toast(error.message, "error"); });
        });
        App.$("#userSearch").addEventListener("keydown", function (event) {
            if (event.key === "Enter") {
                event.preventDefault();
                loadUsers().catch(function (error) { App.toast(error.message, "error"); });
            }
        });

        App.$("#generateForm").addEventListener("submit", generateAccounts);
        App.$("#importForm").addEventListener("submit", importStudents);
        App.$("#importFile").addEventListener("change", loadImportFile);
        App.$("#settingsForm").addEventListener("submit", saveSettings);

        App.$('.tab[data-tab="dashboard"]').addEventListener("click", function () {
            loadStats().catch(function (error) { App.toast(error.message, "error"); });
            loadActivity().catch(function (error) { App.toast(error.message, "error"); });
        });
        App.$('.tab[data-tab="users"]').addEventListener("click", function () {
            loadUsers().catch(function (error) { App.toast(error.message, "error"); });
        });
        App.$('.tab[data-tab="menu"]').addEventListener("click", function () {
            menuUi.refresh().catch(function (error) { App.toast(error.message, "error"); });
        });
        App.$('.tab[data-tab="orders"]').addEventListener("click", function () {
            ordersUi.refresh().catch(function (error) { App.toast(error.message, "error"); });
        });
        App.$('.tab[data-tab="report"]').addEventListener("click", function () {
            reportUi.refresh().catch(function (error) { App.toast(error.message, "error"); });
        });
        App.$("#refreshReportButton").addEventListener("click", function () {
            reportUi.refresh().then(function () { App.toast("Raport odświeżony.", "success"); })
                .catch(function (error) { App.toast(error.message, "error"); });
        });
        App.$('.tab[data-tab="settings"]').addEventListener("click", function () {
            loadSettings().catch(function (error) { App.toast(error.message, "error"); });
        });
        App.$('.tab[data-tab="audit"]').addEventListener("click", function () {
            loadAudit().catch(function (error) { App.toast(error.message, "error"); });
        });

        window.setInterval(function () {
            if (document.hidden) return;
            const dashboardTab = App.$("#tab-dashboard");
            if (dashboardTab && dashboardTab.classList.contains("is-active")) {
                loadStats().catch(function () {});
                loadActivity().catch(function () {});
            }
        }, 20000);

        window.addEventListener("resize", function () {
            if (!activityData) return;
            window.clearTimeout(resizeTimer);
            resizeTimer = window.setTimeout(function () {
                const dashboardTab = App.$("#tab-dashboard");
                if (dashboardTab && dashboardTab.classList.contains("is-active")) {
                    renderActivity(activityData);
                }
            }, 200);
        });
    });

    async function loadStats() {
        const stats = await App.api.get("/api/admin/stats");
        const grid = App.$("#statsGrid");
        App.clear(grid);

        const cards = [
            ["Uczniowie", stats.students, "aktywni"],
            ["Kuchnia", stats.kitchen, "aktywne konta"],
            ["Administratorzy", stats.admins, "aktywne konta"],
            ["Konta łącznie", stats.total_users, "wszystkie role"],
            ["Zamówienia dziś", stats.orders_today, "złożone dzisiaj"],
            ["Otwarte zamówienia", stats.open_orders, "w realizacji"],
            ["Do zapłaty", stats.awaiting_payment, "przy odbiorze"],
            ["Przychód dziś", App.fmtPLN(stats.revenue_today_cents), "opłacone"],
            ["Pozycje menu", stats.menu_available, "dostępne"],
            ["Aktywne sesje", stats.active_sessions, "zalogowani"],
        ];

        cards.forEach(function (card) {
            const node = App.h("div", { class: "stat-card" });
            node.append(
                App.h("span", { class: "stat-label", text: card[0] }),
                App.h("strong", { class: "stat-value", text: card[1] }),
                App.h("span", { class: "stat-hint", text: card[2] })
            );
            grid.append(node);
        });

        App.setText("#dashboardUpdated", "Zaktualizowano " + new Date().toLocaleTimeString("pl-PL"));
    }

    async function loadActivity() {
        const select = App.$("#activityDays");
        const days = select ? select.value : "14";
        const data = await App.api.get("/api/admin/stats/activity?days=" + encodeURIComponent(days));
        activityData = data;
        renderActivity(data);
    }

    function renderActivity(data) {
        if (!window.Charts) {
            App.setText("#activitySummary", "Wykresy są niedostępne w tej przeglądarce.");
            return;
        }

        const daily = Array.isArray(data.daily) ? data.daily : [];
        const dateLabels = daily.map(function (point) {
            return point.date.slice(8, 10) + "." + point.date.slice(5, 7);
        });
        const orders = daily.map(function (point) { return point.orders; });
        const revenue = daily.map(function (point) { return point.revenue_cents; });

        Charts.line(App.$("#chartDailyOrders"), dateLabels, orders, {
            color: getComputedStyle(document.documentElement).getPropertyValue("--primary").trim() || "#227b88",
        });

        Charts.bars(App.$("#chartDailyRevenue"), dateLabels, revenue, {
            color: getComputedStyle(document.documentElement).getPropertyValue("--accent").trim() || "#f0b400",
            valueFormatter: function (value) { return Math.round(value / 100) + " zł"; },
        });

        const hourly = Array.isArray(data.hourly_today) ? data.hourly_today : [];
        Charts.bars(
            App.$("#chartHourly"),
            hourly.map(function (point) { return String(point.hour).padStart(2, "0"); }),
            hourly.map(function (point) { return point.orders; }),
            {
                color: getComputedStyle(document.documentElement).getPropertyValue("--primary").trim() || "#227b88",
                highlightIndex: new Date().getHours(),
            }
        );

        const statusMeta = [
            ["new", "Nowe", "#3b82f6"],
            ["preparing", "W przygotowaniu", "#f59e0b"],
            ["ready", "Gotowe", "#16a34a"],
            ["completed", "Wydane", "#6b7280"],
            ["cancelled", "Anulowane", "#dc2626"],
        ];
        const statusCounts = data.status_counts || {};
        Charts.horizontal(
            App.$("#chartStatus"),
            statusMeta.map(function (item) {
                return {
                    label: item[1],
                    value: Number(statusCounts[item[0]] || 0),
                    color: item[2],
                };
            })
        );

        const topItems = Array.isArray(data.top_items) ? data.top_items : [];
        Charts.horizontal(
            App.$("#chartTopItems"),
            topItems.map(function (item) {
                return {
                    label: item.name,
                    value: item.quantity,
                    display: item.quantity + " szt.",
                    color: getComputedStyle(document.documentElement).getPropertyValue("--primary").trim() || "#227b88",
                };
            })
        );

        const totals = data.totals || {};
        App.setText(
            "#activitySummary",
            data.from + " – " + data.to + " · " +
            (totals.orders || 0) + " zamówień · " +
            App.fmtPLN(totals.revenue_cents || 0) + " utargu · " +
            "średnio " + App.fmtPLN(totals.average_order_cents || 0) + " · " +
            (totals.completed || 0) + " wydanych · " +
            (totals.cancelled || 0) + " anulowanych"
        );
    }

    async function loadUsers() {
        const params = new URLSearchParams();
        const role = App.$("#userRoleFilter").value;
        const active = App.$("#userActiveFilter").value;
        const query = App.$("#userSearch").value.trim();
        if (role) params.set("role", role);
        if (active) params.set("active", active);
        if (query) params.set("q", query);

        const users = await App.api.get("/api/admin/users" + (params.toString() ? "?" + params.toString() : ""));
        const tbody = App.$("#usersTableBody");
        App.clear(tbody);

        if (!users.length) {
            const row = App.h("tr");
            const cell = App.h("td", { class: "empty-state", text: "Brak kont spełniających kryteria." });
            cell.colSpan = 6;
            row.append(cell);
            tbody.append(row);
            return;
        }

        users.forEach(function (user) {
            const row = App.h("tr");

            const loginCell = App.h("td");
            loginCell.append(App.h("code", { text: user.login }));

            const codeCell = App.h("td");
            codeCell.append(App.h("code", { text: user.display_code }));

            const roleCell = App.h("td", { text: App.roleLabels[user.role] || user.role });

            const statusCell = App.h("td");
            statusCell.append(
                user.active
                    ? App.h("span", { class: "badge badge-ready", text: "Aktywne" })
                    : App.h("span", { class: "badge badge-cancelled", text: "Nieaktywne" })
            );
            if (user.must_change_password) {
                statusCell.append(App.h("span", { class: "badge badge-new", text: "Hasło startowe" }));
            }

            const createdCell = App.h("td", { text: App.fmtDate(user.created_at) });

            const actions = App.h("td", { class: "table-actions" });
            actions.append(
                App.h("button", {
                    class: user.active ? "btn btn-danger btn-small" : "btn btn-secondary btn-small",
                    text: user.active ? "Dezaktywuj" : "Aktywuj",
                    attrs: { type: "button" },
                    on: {
                        click: async function () {
                            if (user.active && !window.confirm("Dezaktywować konto " + user.login + "?")) return;
                            try {
                                await App.api.patch("/api/admin/users/" + user.id, { active: !user.active });
                                App.toast("Konto zaktualizowane.", "success");
                                await loadUsers();
                            } catch (error) {
                                App.toast(error.message, "error");
                            }
                        },
                    },
                }),
                App.h("button", {
                    class: "btn btn-ghost btn-small",
                    text: "Reset hasła",
                    attrs: { type: "button" },
                    on: {
                        click: async function () {
                            if (!window.confirm("Wygenerować nowe hasło dla " + user.login + "?")) return;
                            try {
                                const data = await App.api.post(
                                    "/api/admin/users/" + user.id + "/reset-password",
                                    {}
                                );
                                showSecretDialog(
                                    "Nowe hasło",
                                    [[data.login, data.password]],
                                    data.note
                                );
                                await loadUsers();
                            } catch (error) {
                                App.toast(error.message, "error");
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
                            const answer = window.prompt(
                                "Usunięcie konta " + user.login + " jest nieodwracalne. " +
                                "Usunięta zostanie także historia zamówień tego konta. " +
                                "Wpisz login, aby potwierdzić:",
                                ""
                            );
                            if (answer === null) return;
                            if (answer.trim() !== user.login) {
                                App.toast("Login nie pasuje. Konto nie zostało usunięte.", "error");
                                return;
                            }
                            try {
                                const result = await App.api.del("/api/admin/users/" + user.id);
                                App.toast(
                                    "Konto usunięte. Usunięto zamówienia: " +
                                    (result.deleted_orders || 0) + ".",
                                    "success"
                                );
                                await loadUsers();
                                await loadStats();
                                await loadActivity();
                            } catch (error) {
                                App.toast(error.message, "error");
                            }
                        },
                    },
                })
            );

            if (user.role === "student") {
                actions.append(
                    App.h("button", {
                        class: "btn btn-secondary btn-small",
                        text: "Zdejmij blokadę",
                        attrs: { type: "button" },
                        on: {
                            click: async function () {
                                if (!window.confirm(
                                    "Zdjąć blokadę nieodebranych dla " + user.login + "? " +
                                    "Historia zamówień i raporty pozostaną bez zmian."
                                )) return;
                                try {
                                    await App.api.post(
                                        "/api/admin/users/" + user.id + "/clear-uncollected-block",
                                        {}
                                    );
                                    App.toast("Blokada nieodebranych zdjęta.", "success");
                                    await loadUsers();
                                } catch (error) {
                                    App.toast(error.message, "error");
                                }
                            },
                        },
                    })
                );
            }

            row.append(loginCell, codeCell, roleCell, statusCell, createdCell, actions);
            tbody.append(row);
        });
    }

    async function generateAccounts(event) {
        event.preventDefault();
        const button = App.$("#generateButton");
        const role = App.$("#generateRole").value;
        const count = Number.parseInt(App.$("#generateCount").value, 10);

        if (!Number.isFinite(count) || count < 1 || count > 100) {
            App.toast("Liczba kont musi być od 1 do 100.", "error");
            return;
        }

        App.setBusy(button, true, "Generowanie…");
        try {
            const data = await App.api.post("/api/admin/users/generate", { role: role, count: count });
            generatedAccounts = data.accounts || [];
            renderGeneratedAccounts(data.note);
            await loadUsers();
            App.toast("Wygenerowano " + generatedAccounts.length + " kont.", "success");
        } catch (error) {
            App.toast(error.message, "error");
        } finally {
            App.setBusy(button, false, "Generuj");
        }
    }

    let importedAccounts = [];

    async function loadImportFile() {
        const input = App.$("#importFile");
        const file = input.files && input.files[0];
        if (!file) return;
        if (file.size > 64 * 1024) {
            App.toast("Plik jest zbyt duży (maks. 64 KiB).", "error");
            input.value = "";
            return;
        }
        try {
            App.$("#importCsv").value = await file.text();
        } catch (error) {
            App.toast("Nie udało się odczytać pliku.", "error");
        }
    }

    async function importStudents(event) {
        event.preventDefault();
        const button = App.$("#importButton");
        const csv = App.$("#importCsv").value.trim();
        if (!csv) {
            App.toast("Wklej dane CSV lub wybierz plik.", "error");
            return;
        }

        App.setBusy(button, true, "Importowanie…");
        try {
            const response = await fetch("/api/admin/users/import", {
                method: "POST",
                headers: {
                    "Content-Type": "text/csv; charset=utf-8",
                    "X-CSRF-Token": App.state.csrf || "",
                },
                body: csv,
                credentials: "same-origin",
                cache: "no-store",
            });
            const data = await response.json().catch(function () { return null; });
            if (!response.ok) {
                throw new Error((data && data.error) || "Nie udało się zaimportować kont.");
            }
            importedAccounts = data.accounts || [];
            renderAccounts("#importedAccounts", importedAccounts, data.note);
            App.$("#importCsv").value = "";
            App.$("#importFile").value = "";
            await loadUsers();
            App.toast("Zaimportowano " + importedAccounts.length + " kont.", "success");
        } catch (error) {
            App.toast(error.message || "Nie udało się zaimportować kont.", "error");
        } finally {
            App.setBusy(button, false, "Importuj");
        }
    }

    function renderGeneratedAccounts(note) {
        renderAccounts("#generatedAccounts", generatedAccounts, note);
    }

    function renderAccounts(selector, accounts, note) {
        const container = App.$(selector);
        App.clear(container);
        container.hidden = false;

        const warning = App.h("div", { class: "banner banner-warning" });
        warning.append(
            App.h("strong", { text: "Zapisz te dane teraz — hasła nie będą ponownie pokazane. " }),
            App.h("span", { text: note || "" })
        );

        const toolbar = App.h("div", { class: "toolbar" });
        toolbar.append(
            App.h("button", {
                class: "btn btn-secondary btn-small",
                text: "Kopiuj wszystkie",
                attrs: { type: "button" },
                on: {
                    click: function () {
                        copyText(accountsAsText(accounts), "Dane kont skopiowane.");
                    },
                },
            }),
            App.h("button", {
                class: "btn btn-ghost btn-small",
                text: "Pobierz CSV",
                attrs: { type: "button" },
                on: { click: function () { downloadCsv(accounts); } },
            })
        );

        const showGroup = accounts.some(function (account) { return account.group; });
        const table = App.h("table", { class: "table" });
        const thead = App.h("thead");
        const headRow = App.h("tr");
        const headers = ["Login", "Hasło", "Rola", "Kod"];
        if (showGroup) headers.push("Grupa");
        headers.forEach(function (label) {
            headRow.append(App.h("th", { text: label }));
        });
        thead.append(headRow);

        const tbody = App.h("tbody");
        accounts.forEach(function (account) {
            const row = App.h("tr");
            row.append(
                App.h("td", {}, [App.h("code", { text: account.login })]),
                App.h("td", {}, [App.h("code", { text: account.password })]),
                App.h("td", { text: App.roleLabels[account.role] || account.role }),
                App.h("td", {}, [App.h("code", { text: account.display_code })])
            );
            if (showGroup) {
                row.append(App.h("td", { text: account.group || "" }));
            }
            tbody.append(row);
        });
        table.append(thead, tbody);

        const wrapper = App.h("div", { class: "table-wrap" });
        wrapper.append(table);

        container.append(warning, toolbar, wrapper);
    }

    function accountsAsText(accounts) {
        return accounts.map(function (account) {
            const base = account.login + "\t" + account.password + "\t" + (App.roleLabels[account.role] || account.role) + "\t" + account.display_code;
            return account.group ? base + "\t" + account.group : base;
        }).join("\n");
    }

    function downloadCsv(accounts) {
        const header = ["login", "haslo", "rola", "kod"];
        if (accounts.some(function (account) { return account.group; })) {
            header.push("grupa");
        }
        const lines = [header.join(";")];
        accounts.forEach(function (account) {
            const cells = [
                csvCell(account.login),
                csvCell(account.password),
                csvCell(App.roleLabels[account.role] || account.role),
                csvCell(account.display_code),
            ];
            if (header.length > 4) {
                cells.push(csvCell(account.group || ""));
            }
            lines.push(cells.join(";"));
        });

        const blob = new Blob(["\ufeff" + lines.join("\r\n")], { type: "text/csv;charset=utf-8" });
        downloadBlob(blob, "konta-zsl-order-" + new Date().toISOString().slice(0, 10) + ".csv");
        App.toast("Plik CSV został pobrany. Przechowuj go bezpiecznie.", "success");
    }

    async function exportUsersCsv() {
        const params = new URLSearchParams();
        const role = App.$("#userRoleFilter").value;
        const active = App.$("#userActiveFilter").value;
        const query = App.$("#userSearch").value.trim();
        if (role) params.set("role", role);
        if (active) params.set("active", active);
        if (query) params.set("q", query);

        const path = "/api/admin/users/export.csv" +
            (params.toString() ? "?" + params.toString() : "");

        try {
            const response = await fetch(path, {
                credentials: "same-origin",
                cache: "no-store",
            });

            if (!response.ok) {
                if (response.status === 401) {
                    window.location.href = "/login";
                    return;
                }
                let message = "Nie udało się wyeksportować kont.";
                try {
                    const data = await response.json();
                    if (data && data.error) message = data.error;
                } catch (error) {
                    // Odpowiedź nie była JSON-em.
                }
                throw new Error(message);
            }

            const blob = await response.blob();
            const disposition = response.headers.get("content-disposition") || "";
            const match = disposition.match(/filename="([^"]+)"/);
            const filename = match ? match[1] : "konta-zsl-order.csv";
            downloadBlob(blob, filename);
            App.toast(
                "Wyeksportowano metadane kont. Hasła nie są częścią eksportu.",
                "success"
            );
        } catch (error) {
            App.toast(error.message || "Nie udało się wyeksportować kont.", "error");
        }
    }

    function downloadBlob(blob, filename) {
        const url = URL.createObjectURL(blob);
        const link = document.createElement("a");
        link.href = url;
        link.download = filename;
        document.body.append(link);
        link.click();
        link.remove();
        window.setTimeout(function () { URL.revokeObjectURL(url); }, 1000);
    }

    function csvCell(value) {
        const text = String(value == null ? "" : value);
        return '"' + text.replace(/"/g, '""') + '"';
    }

    function showSecretDialog(title, rows, note) {
        const dialog = App.h("dialog", { class: "dialog" });
        dialog.append(App.h("h2", { text: title }));
        if (note) {
            dialog.append(App.h("p", { class: "muted small", text: note }));
        }

        const box = App.h("div", { class: "secret-box" });
        rows.forEach(function (row) {
            const line = App.h("div", { class: "secret-line" });
            line.append(
                App.h("span", { class: "muted tiny", text: "Login" }),
                App.h("code", { text: row[0] }),
                App.h("span", { class: "muted tiny", text: "Hasło" }),
                App.h("code", { text: row[1] })
            );
            box.append(line);
        });
        dialog.append(box);

        const actions = App.h("div", { class: "dialog-actions" });
        actions.append(
            App.h("button", {
                class: "btn btn-secondary",
                text: "Kopiuj",
                attrs: { type: "button" },
                on: {
                    click: function () {
                        copyText(rows.map(function (row) { return row[0] + "\t" + row[1]; }).join("\n"),
                            "Dane skopiowane.");
                    },
                },
            }),
            App.h("button", {
                class: "btn btn-primary",
                text: "Zamknij",
                attrs: { type: "button" },
                on: { click: function () { dialog.close(); } },
            })
        );
        dialog.append(actions);
        dialog.addEventListener("close", function () { dialog.remove(); });
        document.body.append(dialog);
        dialog.showModal();
    }

    async function copyText(text, successMessage) {
        try {
            await navigator.clipboard.writeText(text);
            App.toast(successMessage || "Skopiowano.", "success");
        } catch (error) {
            App.toast("Nie udało się skopiować. Zaznacz tekst ręcznie.", "error");
        }
    }

    async function loadSettings() {
        const settings = await App.api.get("/api/admin/settings");
        App.$("#settingCafeteriaName").value = settings.cafeteria_name || "Stołówka ZSŁ";
        App.$("#settingOrderingEnabled").checked = settings.ordering_enabled === "1";
        App.$("#settingPaymentInfo").value = settings.payment_info || "";
        App.$("#settingMaxOpenOrders").value = settings.max_open_orders || "3";
        App.$("#settingMaxOrderItems").value = settings.max_order_items || "20";
        App.$("#settingCancelWindow").value = settings.cancel_window_minutes || "30";
        App.$("#settingPickupLimit").value = settings.pickup_limit_minutes || "120";
        App.$("#settingMaxUncollected").value = settings.max_uncollected || "0";
        App.$("#settingsError").hidden = true;
    }

    async function saveSettings(event) {
        event.preventDefault();
        const button = App.$("#saveSettingsButton");
        const errorNode = App.$("#settingsError");
        errorNode.hidden = true;

        const payload = {
            cafeteria_name: App.$("#settingCafeteriaName").value.trim(),
            ordering_enabled: App.$("#settingOrderingEnabled").checked,
            payment_info: App.$("#settingPaymentInfo").value.trim(),
            max_open_orders: Number.parseInt(App.$("#settingMaxOpenOrders").value, 10),
            max_order_items: Number.parseInt(App.$("#settingMaxOrderItems").value, 10),
            cancel_window_minutes: Number.parseInt(App.$("#settingCancelWindow").value, 10),
            pickup_limit_minutes: Number.parseInt(App.$("#settingPickupLimit").value, 10),
            max_uncollected: Number.parseInt(App.$("#settingMaxUncollected").value, 10),
        };

        App.setBusy(button, true, "Zapisywanie…");
        try {
            await App.api.put("/api/admin/settings", payload);
            App.setText("#cafeteriaName", payload.cafeteria_name);
            App.toast("Ustawienia zapisane.", "success");
            await loadSettings();
        } catch (error) {
            errorNode.textContent = error.message;
            errorNode.hidden = false;
        } finally {
            App.setBusy(button, false, "Zapisz ustawienia");
        }
    }

    async function loadAudit() {
        const entries = await App.api.get("/api/admin/audit?limit=200");
        const tbody = App.$("#auditTableBody");
        App.clear(tbody);

        if (!entries.length) {
            const row = App.h("tr");
            const cell = App.h("td", { class: "empty-state", text: "Brak zdarzeń." });
            cell.colSpan = 4;
            row.append(cell);
            tbody.append(row);
            return;
        }

        entries.forEach(function (entry) {
            const row = App.h("tr");
            row.append(
                App.h("td", { text: App.fmtDate(entry.created_at) }),
                App.h("td", {}, [
                    entry.actor_login
                        ? App.h("code", { text: entry.actor_login })
                        : App.h("span", { class: "muted", text: "system" }),
                ]),
                App.h("td", {}, [App.h("code", { text: entry.action })]),
                App.h("td", { class: "muted small", text: entry.details })
            );
            tbody.append(row);
        });
    }
})();
