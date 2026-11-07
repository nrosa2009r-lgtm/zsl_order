/* ZSL Order — logowanie. */
(function () {
    "use strict";

    document.addEventListener("DOMContentLoaded", async function () {
        const form = App.$("#loginForm");
        const errorNode = App.$("#loginError");
        const button = App.$("#loginButton");
        const loginInput = App.$("#loginInput");

        await App.loadPublicInfo();

        // Jeśli użytkownik ma już ważną sesję, od razu przechodzimy do panelu.
        try {
            const data = await App.api.get("/api/auth/me", { redirectOn401: false });
            App.state.csrf = data.csrf_token;
            App.state.user = data.user;
            window.location.replace(App.rolePath(data.user.role));
            return;
        } catch (error) {
            // Brak sesji jest tu stanem oczekiwanym.
        }

        form.addEventListener("submit", async function (event) {
            event.preventDefault();
            errorNode.hidden = true;

            const login = loginInput.value.trim();
            const password = App.$("#passwordInput").value;

            if (!login || !password) {
                errorNode.textContent = "Wpisz login i hasło.";
                errorNode.hidden = false;
                return;
            }

            App.setBusy(button, true, "Logowanie…");
            try {
                const data = await App.api.post(
                    "/api/auth/login",
                    { login: login, password: password },
                    { redirectOn401: false }
                );
                App.state.csrf = data.csrf_token;
                App.state.user = data.user;
                window.location.replace(App.rolePath(data.user.role));
            } catch (error) {
                errorNode.textContent = error.message;
                errorNode.hidden = false;
                App.$("#passwordInput").value = "";
                App.$("#passwordInput").focus();
            } finally {
                App.setBusy(button, false);
            }
        });
    });
})();
