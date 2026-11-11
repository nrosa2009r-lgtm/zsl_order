/* ZSL Order — wspólne funkcje UI i bezpiecznego wywoływania API. */
(function () {
    "use strict";

    const state = {
        csrf: null,
        user: null,
        publicInfo: null,
    };

    class ApiError extends Error {
        constructor(message, status, code) {
            super(message);
            this.name = "ApiError";
            this.status = status || 0;
            this.code = code || "";
        }
    }

    async function request(path, options) {
        options = options || {};
        const method = (options.method || "GET").toUpperCase();
        const headers = Object.assign({}, options.headers || {});
        let body = options.body;

        if (body !== undefined) {
            headers["Content-Type"] = "application/json";
            body = JSON.stringify(body);
        }
        if (!["GET", "HEAD", "OPTIONS"].includes(method) && state.csrf) {
            headers["X-CSRF-Token"] = state.csrf;
        }

        let response;
        try {
            response = await fetch(path, {
                method: method,
                headers: headers,
                body: body,
                credentials: "same-origin",
                cache: "no-store",
            });
        } catch (error) {
            throw new ApiError("Brak połączenia z serwerem. Sprawdź sieć.", 0, "network");
        }

        let data = null;
        const contentType = response.headers.get("content-type") || "";
        if (contentType.includes("application/json")) {
            data = await response.json().catch(function () { return null; });
        }

        if (!response.ok) {
            const error = new ApiError(
                (data && data.error) || "Wystąpił błąd. Spróbuj ponownie.",
                response.status,
                (data && data.code) || ""
            );

            if (response.status === 401 && options.redirectOn401 !== false) {
                if (window.location.pathname !== "/login") {
                    window.location.href = "/login";
                }
            }
            if (error.code === "password_change_required") {
                openPasswordDialog(true);
            }
            throw error;
        }

        return data;
    }

    const api = {
        get: function (path, options) {
            return request(path, Object.assign({}, options, { method: "GET" }));
        },
        post: function (path, body, options) {
            return request(path, Object.assign({}, options, { method: "POST", body: body }));
        },
        put: function (path, body, options) {
            return request(path, Object.assign({}, options, { method: "PUT", body: body }));
        },
        patch: function (path, body, options) {
            return request(path, Object.assign({}, options, { method: "PATCH", body: body }));
        },
        del: function (path, options) {
            return request(path, Object.assign({}, options, { method: "DELETE" }));
        },
        request: request,
    };

    function $(selector, root) {
        return (root || document).querySelector(selector);
    }

    function $$(selector, root) {
        return Array.prototype.slice.call((root || document).querySelectorAll(selector));
    }

    function h(tag, options, children) {
        const node = document.createElement(tag);
        options = options || {};
        if (options.class) node.className = options.class;
        if (options.text !== undefined) node.textContent = String(options.text);
        if (options.attrs) {
            Object.keys(options.attrs).forEach(function (key) {
                const value = options.attrs[key];
                if (value === false || value === null || value === undefined) return;
                node.setAttribute(key, value === true ? "" : String(value));
            });
        }
        if (options.on) {
            Object.keys(options.on).forEach(function (eventName) {
                node.addEventListener(eventName, options.on[eventName]);
            });
        }
        (children || []).forEach(function (child) {
            if (child === null || child === undefined) return;
            node.append(child instanceof Node ? child : document.createTextNode(String(child)));
        });
        return node;
    }

    function clear(node) {
        while (node && node.firstChild) node.removeChild(node.firstChild);
    }

    function setText(selector, text) {
        const node = $(selector);
        if (node) node.textContent = text;
    }

    function toast(message, type) {
        const container = $("#toastContainer");
        if (!container) return;
        const item = h("div", {
            class: "toast toast-" + (type || "info"),
            attrs: { role: "status" },
        }, [message]);
        container.append(item);
        window.setTimeout(function () {
            item.classList.add("is-leaving");
            window.setTimeout(function () { item.remove(); }, 250);
        }, type === "error" ? 6500 : 4200);
    }

    function fmtPLN(cents) {
        return (Number(cents || 0) / 100).toLocaleString("pl-PL", {
            style: "currency",
            currency: "PLN",
        });
    }

    function fmtDate(timestamp) {
        if (!timestamp) return "—";
        return new Date(Number(timestamp) * 1000).toLocaleString("pl-PL", {
            dateStyle: "short",
            timeStyle: "short",
            timeZone: "Europe/Warsaw",
        });
    }

    function fmtTime(timestamp) {
        if (!timestamp) return "—";
        return new Date(Number(timestamp) * 1000).toLocaleTimeString("pl-PL", {
            hour: "2-digit",
            minute: "2-digit",
            timeZone: "Europe/Warsaw",
        });
    }

    const statusLabels = {
        new: "Nowe",
        preparing: "W przygotowaniu",
        ready: "Gotowe",
        completed: "Wydane",
        cancelled: "Anulowane",
        uncollected: "Nieodebrane",
    };

    const paymentLabels = {
        cash: "Gotówka",
        card: "Karta",
        unpaid: "Nieopłacone",
        paid: "Opłacone",
    };

    const roleLabels = {
        student: "Uczeń",
        kitchen: "Kuchnia",
        admin: "Administrator",
    };

    function statusBadge(status) {
        return h("span", {
            class: "badge badge-" + status,
            text: statusLabels[status] || status,
        });
    }

    function paymentBadge(status) {
        return h("span", {
            class: "badge badge-pay-" + status,
            text: paymentLabels[status] || status,
        });
    }

    function rolePath(role) {
        if (role === "kitchen") return "/kitchen";
        if (role === "admin") return "/admin";
        return "/student";
    }

    function setupTabs(onChange) {
        const tabs = $$(".tab[data-tab]");
        if (!tabs.length) return;
        tabs.forEach(function (tab) {
            tab.addEventListener("click", function () {
                const name = tab.getAttribute("data-tab");
                tabs.forEach(function (other) {
                    other.classList.toggle("is-active", other === tab);
                    other.setAttribute("aria-selected", other === tab ? "true" : "false");
                });
                $$(".tab-panel").forEach(function (panel) {
                    panel.classList.toggle("is-active", panel.id === "tab-" + name);
                });
                if (typeof onChange === "function") onChange(name);
            });
        });
    }

    function setupDialogs() {
        $$("[data-close-dialog]").forEach(function (button) {
            button.addEventListener("click", function () {
                const dialog = button.closest("dialog");
                if (dialog) dialog.close();
            });
        });
        $$("dialog").forEach(function (dialog) {
            dialog.addEventListener("click", function (event) {
                if (event.target === dialog) dialog.close();
            });
        });
    }

    function applyUser(user) {
        state.user = user;
        const label = user.login + " · " + (roleLabels[user.role] || user.role);
        setText("#userInfo", label);
        setText("#accountLogin", user.login);
        setText("#accountCode", user.display_code);
    }

    async function loadPublicInfo() {
        try {
            const info = await api.get("/api/public/info", { redirectOn401: false });
            state.publicInfo = info;
            setText("#cafeteriaName", info.cafeteria_name || "Stołówka ZSŁ");
            return info;
        } catch (error) {
            return null;
        }
    }

    async function initSession() {
        const data = await api.get("/api/auth/me", { redirectOn401: false });
        state.csrf = data.csrf_token;
        state.user = data.user;
        return data.user;
    }

    function bootPage(expectedRole, callback) {
        document.addEventListener("DOMContentLoaded", async function () {
            setupDialogs();
            setupTabs();
            await loadPublicInfo();

            const logoutButton = $("#logoutButton");
            if (logoutButton) {
                logoutButton.addEventListener("click", async function () {
                    try {
                        await api.post("/api/auth/logout", {});
                    } catch (error) {
                        // Nawet jeśli sesja już wygasła, wracamy do logowania.
                    }
                    window.location.href = "/login";
                });
            }

            const changePasswordButton = $("#changePasswordButton");
            if (changePasswordButton) {
                changePasswordButton.addEventListener("click", function () {
                    openPasswordDialog(false);
                });
            }

            let user;
            try {
                user = await initSession();
            } catch (error) {
                window.location.replace("/login");
                return;
            }

            if (user.role !== expectedRole) {
                window.location.replace(rolePath(user.role));
                return;
            }

            applyUser(user);
            if (user.must_change_password) {
                // Wymuszona zmiana hasła startowego. Po zmianie strona się
                // przeładuje i callback uruchomi się ponownie.
                openPasswordDialog(true);
                return;
            }

            try {
                await callback(user);
            } catch (error) {
                if (error.status === 401) {
                    window.location.replace("/login");
                    return;
                }
                toast(error.message || "Nie udało się załadować danych.", "error");
            }
        });
    }

    function openPasswordDialog(force) {
        let dialog = $("#passwordDialog");
        if (!dialog) {
            dialog = h("dialog", { class: "dialog", attrs: { id: "passwordDialog" } });
            const form = h("form", { class: "stack", attrs: { method: "dialog" } });
            form.append(
                h("h2", { text: "Zmiana hasła" }),
                h("p", {
                    class: "muted small",
                    text: "Hasło musi mieć co najmniej 10 znaków, literę i cyfrę.",
                })
            );

            const fields = [
                ["currentPassword", "Obecne hasło", "current-password"],
                ["newPassword", "Nowe hasło", "new-password"],
                ["confirmPassword", "Powtórz nowe hasło", "new-password"],
            ];
            fields.forEach(function (field) {
                const id = "pw_" + field[0];
                const wrapper = h("div", { class: "field" });
                wrapper.append(
                    h("label", { text: field[1], attrs: { for: id } }),
                    h("input", {
                        attrs: {
                            id: id,
                            name: field[0],
                            type: "password",
                            autocomplete: field[2],
                            maxlength: "128",
                            required: true,
                        },
                    })
                );
                form.append(wrapper);
            });

            form.append(
                h("p", {
                    class: "form-error",
                    attrs: { id: "passwordError", role: "alert", hidden: true },
                })
            );

            const actions = h("div", { class: "dialog-actions" });
            const cancel = h("button", {
                class: "btn btn-ghost",
                text: "Anuluj",
                attrs: { type: "button", id: "passwordCancel" },
            });
            cancel.addEventListener("click", function () {
                if (!dialog.dataset.force) dialog.close();
            });
            const submit = h("button", {
                class: "btn btn-primary",
                text: "Zmień hasło",
                attrs: { type: "submit", id: "passwordSubmit" },
            });
            actions.append(cancel, submit);
            form.append(actions);
            dialog.append(form);

            dialog.addEventListener("cancel", function (event) {
                if (dialog.dataset.force) event.preventDefault();
            });

            form.addEventListener("submit", async function (event) {
                event.preventDefault();
                const errorNode = $("#passwordError");
                errorNode.hidden = true;
                const current = $("#pw_currentPassword").value;
                const next = $("#pw_newPassword").value;
                const confirm = $("#pw_confirmPassword").value;

                if (next !== confirm) {
                    errorNode.textContent = "Nowe hasła nie są identyczne.";
                    errorNode.hidden = false;
                    return;
                }
                if (next.length < 10) {
                    errorNode.textContent = "Nowe hasło musi mieć co najmniej 10 znaków.";
                    errorNode.hidden = false;
                    return;
                }

                submit.disabled = true;
                submit.textContent = "Zapisywanie…";
                try {
                    await api.post("/api/auth/change-password", {
                        current_password: current,
                        new_password: next,
                    });
                    const wasForced = dialog.dataset.force === "1";
                    delete dialog.dataset.force;
                    dialog.close();
                    form.reset();
                    toast("Hasło zostało zmienione.", "success");
                    if (wasForced) {
                        window.setTimeout(function () { window.location.reload(); }, 350);
                    }
                } catch (error) {
                    errorNode.textContent = error.message;
                    errorNode.hidden = false;
                } finally {
                    submit.disabled = false;
                    submit.textContent = "Zmień hasło";
                }
            });

            document.body.append(dialog);
        }

        if (force) {
            dialog.dataset.force = "1";
        } else {
            delete dialog.dataset.force;
        }
        if (typeof dialog.showModal === "function") {
            if (!dialog.open) dialog.showModal();
        } else {
            toast("Twoja przeglądarka nie obsługuje okien dialogowych.", "error");
        }
    }

    function setBusy(button, busy, busyText) {
        if (!button) return;
        if (busy) {
            if (!button.dataset.originalText) button.dataset.originalText = button.textContent;
            button.disabled = true;
            button.textContent = busyText || "Przetwarzanie…";
        } else {
            button.disabled = false;
            if (button.dataset.originalText) {
                button.textContent = button.dataset.originalText;
                delete button.dataset.originalText;
            }
        }
    }

    window.App = {
        api: api,
        ApiError: ApiError,
        state: state,
        $: $,
        $$: $$,
        h: h,
        clear: clear,
        setText: setText,
        toast: toast,
        fmtPLN: fmtPLN,
        fmtDate: fmtDate,
        fmtTime: fmtTime,
        statusLabels: statusLabels,
        paymentLabels: paymentLabels,
        roleLabels: roleLabels,
        statusBadge: statusBadge,
        paymentBadge: paymentBadge,
        rolePath: rolePath,
        setupTabs: setupTabs,
        setupDialogs: setupDialogs,
        bootPage: bootPage,
        openPasswordDialog: openPasswordDialog,
        setBusy: setBusy,
        loadPublicInfo: loadPublicInfo,
    };
})();
