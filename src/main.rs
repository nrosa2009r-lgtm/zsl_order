use axum::{
    Router,
    extract::{Request, State},
    http::{HeaderValue, header},
    middleware::{self, Next},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, patch, post, put},
};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::services::ServeDir;

mod auth;
mod client_ip;
mod config;
mod db;
mod error;
mod guard;
mod handlers;
mod logging;
mod security;
mod state;
mod util;

use crate::config::Config;
use crate::error::AppError;
use crate::state::AppState;

pub const CONF_PATH: &str = "config.json";

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("Błąd krytyczny: {error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let _log_guard = logging::init()?;
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        "Uruchamianie ZSL Order"
    );

    let config = Config::load_or_create(CONF_PATH)?;
    println!("ZSL ORDER — system stołówki szkolnej");
    println!("Konfiguracja: {} + .env", CONF_PATH);
    println!("Tryb wdrożenia: {}", config.mode.as_str());
    tracing::info!(
        mode = config.mode.as_str(),
        bind = %config.bind_addr,
        public_url = config
            .public_url
            .as_ref()
            .map(|url| url.origin())
            .unwrap_or("brak"),
        max_connections = config.max_connections,
        session_hours = config.session_hours,
        secure_cookies = config.secure_cookies,
        cookie_secure_setting = config.cookie_secure_setting.as_str(),
        trusted_proxies = config.trusted_proxies.len(),
        "Wczytano konfigurację"
    );

    println!("Inicjalizacja bazy danych...");
    let pool = db::init(&config).await?;
    println!("Baza danych gotowa.");
    tracing::info!("Baza danych gotowa");

    let dummy_hash = security::hash_password("timing-equalization-dummy-password")
        .map_err(|error| format!("Nie udało się przygotować zabezpieczenia logowania: {error}"))?;
    let jwt_secret =
        security::load_or_create_jwt_secret(std::path::Path::new(security::JWT_SECRET_FILE))?;
    tracing::info!(
        file = security::JWT_SECRET_FILE,
        "Sekret JWT wczytany (pliki 0600)"
    );
    let state = Arc::new(AppState {
        db: pool,
        cfg: config.clone(),
        dummy_hash,
        login_limiter: auth::LoginLimiter::new(),
        jwt_secret,
        client_ip_warning: std::sync::Mutex::new(None),
    });

    let app = build_router(state);
    let configured_address = config.bind_address();

    let listener = match TcpListener::bind(configured_address).await {
        Ok(listener) => listener,
        Err(error) => {
            tracing::error!(
                address = %configured_address,
                error = %error,
                "Nie można uruchomić nasłuchu"
            );
            return Err(error.into());
        }
    };

    let local = listener.local_addr()?;
    if let Some(public_url) = &config.public_url {
        println!("Publiczny adres: {}", public_url.origin());
    }
    println!("Serwer działa pod adresem: http://{local}");
    println!("Panel ucznia:  http://{local}/student");
    println!("Panel kuchni:  http://{local}/kitchen");
    println!("Panel admina:  http://{local}/admin");
    println!("Naciśnij Ctrl+C, aby zatrzymać.");
    tracing::info!(address = %local, "Serwer uruchomiony");

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;

    Ok(())
}

fn build_router(state: Arc<AppState>) -> Router {
    let student_routes = Router::new()
        .route("/menu", get(handlers::menu::student_menu))
        .route(
            "/orders",
            get(handlers::orders::student_orders).post(handlers::orders::create_order),
        )
        .route("/orders/:id/cancel", post(handlers::orders::cancel_order))
        .route("/orders/:id/qr.svg", get(handlers::orders::order_qr))
        .route_layer(middleware::from_fn(auth::require_student));

    let staff_routes = Router::new()
        .route(
            "/menu",
            get(handlers::menu::staff_menu).post(handlers::menu::create_menu_item),
        )
        .route(
            "/menu/:id",
            put(handlers::menu::update_menu_item).delete(handlers::menu::hide_menu_item),
        )
        .route("/orders", get(handlers::orders::staff_orders))
        .route("/orders/:id", patch(handlers::orders::staff_update_order))
        .route("/pickup", post(handlers::orders::staff_pickup))
        .route_layer(middleware::from_fn(auth::require_staff));

    let admin_routes = Router::new()
        .route("/stats", get(handlers::admin::stats))
        .route("/stats/activity", get(handlers::admin::activity))
        .route("/users", get(handlers::admin::users))
        .route("/users/export.csv", get(handlers::admin::export_users_csv))
        .route("/users/generate", post(handlers::admin::generate_users))
        .route(
            "/users/:id",
            patch(handlers::admin::update_user).delete(handlers::admin::delete_user),
        )
        .route(
            "/users/:id/reset-password",
            post(handlers::admin::reset_password),
        )
        .route(
            "/settings",
            get(handlers::admin::settings).put(handlers::admin::update_settings),
        )
        .route("/audit", get(handlers::admin::audit_log))
        .route_layer(middleware::from_fn(auth::require_admin));

    let protected_api = Router::new()
        .route("/auth/me", get(handlers::auth::me))
        .route("/auth/logout", post(handlers::auth::logout))
        .route(
            "/auth/change-password",
            post(handlers::auth::change_password),
        )
        .nest("/student", student_routes)
        .nest("/staff", staff_routes)
        .nest("/admin", admin_routes)
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth::auth_middleware,
        ));

    let public_api = Router::new()
        .route("/auth/login", post(handlers::auth::login))
        .route("/auth/token", post(handlers::auth::token))
        .route("/public/info", get(handlers::public::info));

    let api = public_api.merge(protected_api);

    Router::new()
        .route("/", get(index_page))
        .route("/login", get(login_page))
        .route("/student", get(student_page))
        .route("/kitchen", get(kitchen_page))
        .route("/admin", get(admin_page))
        .nest("/api", api)
        .fallback_service(ServeDir::new("frontend").append_index_html_on_directories(true))
        .layer(RequestBodyLimitLayer::new(64 * 1024))
        .layer(middleware::from_fn_with_state(state.clone(), origin_guard))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            security_headers,
        ))
        .layer(middleware::from_fn(logging::request_log))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            client_ip::client_ip_middleware,
        ))
        .with_state(state)
}

async fn index_page() -> Redirect {
    Redirect::to("/login")
}

async fn login_page() -> Html<&'static str> {
    Html(include_str!("../frontend/login.html"))
}

async fn student_page() -> Html<&'static str> {
    Html(include_str!("../frontend/student.html"))
}

async fn kitchen_page() -> Html<&'static str> {
    Html(include_str!("../frontend/kitchen.html"))
}

async fn admin_page() -> Html<&'static str> {
    Html(include_str!("../frontend/admin.html"))
}

/// Nagłówki bezpieczeństwa dla wszystkich odpowiedzi. CSP nie dopuszcza
/// żadnych skryptów inline ani zasobów z zewnętrznych domen.
async fn security_headers(
    State(state): State<Arc<AppState>>,
    request: Request,
    next: Next,
) -> Response {
    let is_api = request.uri().path().starts_with("/api/");
    let mut response = next.run(request).await;
    let headers = response.headers_mut();

    let csp = if state.cfg.is_tunnel() {
        "default-src 'self'; \
         script-src 'self'; \
         style-src 'self'; \
         img-src 'self' data: blob:; \
         font-src 'self'; \
         connect-src 'self'; \
         object-src 'none'; \
         base-uri 'none'; \
         form-action 'self'; \
         frame-ancestors 'none'; \
         upgrade-insecure-requests"
    } else {
        "default-src 'self'; \
         script-src 'self'; \
         style-src 'self'; \
         img-src 'self' data: blob:; \
         font-src 'self'; \
         connect-src 'self'; \
         object-src 'none'; \
         base-uri 'none'; \
         form-action 'self'; \
         frame-ancestors 'none'"
    };
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(csp),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(
        "permissions-policy",
        HeaderValue::from_static("camera=(self), microphone=(), geolocation=()"),
    );
    headers.insert(
        "cross-origin-opener-policy",
        HeaderValue::from_static("same-origin"),
    );
    headers.insert(
        "cross-origin-resource-policy",
        HeaderValue::from_static("same-origin"),
    );

    if state.cfg.is_tunnel() {
        headers.insert(
            header::STRICT_TRANSPORT_SECURITY,
            HeaderValue::from_static("max-age=31536000"),
        );
    }

    if is_api {
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }

    response
}

/// Ochrona przed CSRF na poziomie pochodzenia żądania (dodatkowo do tokenu
/// CSRF sprawdzanego w middleware sesji). W trybie tunnel nagłówek Host
/// musi wskazywać PUBLIC_URL albo adres pętlowy; w trybie local Host jest
/// przyjmowany, ale Origin musi zgadzać się z Host lub PUBLIC_URL.
async fn origin_guard(
    State(state): State<Arc<AppState>>,
    request: Request,
    next: Next,
) -> Result<Response, AppError> {
    let host = request
        .headers()
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");

    if !guard::host_allowed(&state.cfg, host) {
        return Err(AppError::forbidden("Nieprawidłowy nagłówek Host."));
    }

    let method = request.method();
    let safe = *method == axum::http::Method::GET
        || *method == axum::http::Method::HEAD
        || *method == axum::http::Method::OPTIONS;

    if !safe {
        if let Some(site) = request
            .headers()
            .get("sec-fetch-site")
            .and_then(|value| value.to_str().ok())
            && site.eq_ignore_ascii_case("cross-site")
        {
            return Err(AppError::forbidden(
                "Żądanie z innej strony zostało zablokowane.",
            ));
        }

        if let Some(origin) = request
            .headers()
            .get(header::ORIGIN)
            .and_then(|value| value.to_str().ok())
            && !guard::origin_allowed(&state.cfg, host, origin)
        {
            return Err(AppError::forbidden(
                "Żądanie z innej strony zostało zablokowane.",
            ));
        }
    }

    Ok(next.run(request).await)
}

/// Bezpieczne odpowiedzi dla nieznanych ścieżek API (nie zwracają HTML).
#[allow(dead_code)]
async fn api_not_found() -> impl IntoResponse {
    (
        axum::http::StatusCode::NOT_FOUND,
        axum::Json(serde_json::json!({
            "error": "Nie znaleziono zasobu.",
            "code": "not_found"
        })),
    )
}
