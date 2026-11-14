# SPDX-License-Identifier: MIT
# Obraz produkcyjny ZSL Order: kompilacja w etapie 1, minimalny runtime w etapie 2.
# Budowanie: docker build -t zsl-order:1.0.0 .
# Uruchomienie: docker run -d -p 3000:3000 -v zsl-data:/data zsl-order:1.0.0

# ---------- etap 1: kompilacja ----------
FROM rust:1.98-slim-bookworm AS builder

WORKDIR /build

# Migracje są wbudowywane w binarkę makrem `sqlx::migrate!`, więc muszą być
# obecne już na etapie kompilacji.
COPY Cargo.toml Cargo.lock ./
COPY migrations ./migrations
COPY src ./src
COPY frontend ./frontend

RUN cargo build --release --locked --bin zsl-order

# ---------- etap 2: runtime ----------
FROM debian:bookworm-slim AS runtime

# curl jest potrzebny wyłącznie do HEALTHCHECK.
RUN apt-get update \
    && apt-get install -y --no-install-recommends curl ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --system --gid 10001 zsl-order \
    && useradd --system --uid 10001 --gid zsl-order --home-dir /data \
        --shell /usr/sbin/nologin zsl-order \
    && mkdir -p /app /data \
    && chown zsl-order:zsl-order /data

COPY --from=builder /build/target/release/zsl-order /app/zsl-order
COPY --from=builder /build/frontend /app/frontend
COPY deploy/docker/entrypoint.sh /usr/local/bin/zsl-entrypoint

RUN chmod 0755 /app/zsl-order /usr/local/bin/zsl-entrypoint \
    && chown -R root:root /app

# Katalog roboczy = katalog danych. Tu powstają baza, jwt.key, config.json
# i admin_start.txt. Powinien być montowany jako wolumen.
WORKDIR /data

ENV BIND_ADDR=0.0.0.0:3000 \
    DEPLOY_MODE=local \
    RUST_LOG=info,sqlx=warn,hyper=warn,axum=warn

EXPOSE 3000
VOLUME ["/data"]

HEALTHCHECK --interval=30s --timeout=5s --start-period=15s --retries=3 \
    CMD curl -fsS http://127.0.0.1:3000/api/healthz || exit 1

USER zsl-order

ENTRYPOINT ["/usr/local/bin/zsl-entrypoint"]
CMD ["/app/zsl-order"]
