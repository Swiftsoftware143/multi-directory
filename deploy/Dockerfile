# Multi-Directory — container image (handover / self-hosted path, kanban B85).
#
# Builds the application from THIS checkout. The build context must be the repository root:
#   docker build -f deploy/Dockerfile -t multidirectory:latest .
FROM rust:1.82-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release --bin multidirectory

FROM debian:bookworm-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates libssl3 curl \
 && rm -rf /var/lib/apt/lists/*
RUN useradd -r -u 9999 -m multidirectory
WORKDIR /app
COPY --from=build /src/target/release/multidirectory /app/multidirectory
COPY frontend /app/frontend
COPY templates /app/templates
COPY src/migrations /app/src/migrations
RUN mkdir -p /app/uploads && chown -R multidirectory:multidirectory /app
USER multidirectory
ENV APP_HOST=0.0.0.0 \
    APP_PORT=8089 \
    TEMPLATE_DIR=/app/templates \
    MD_UPLOADS_ROOT=/app/uploads \
    RUST_LOG=info
EXPOSE 8089
# The app applies ./src/migrations itself at boot, so a fresh database needs no extra step —
# but it still has NO login row: run `multidirectory create-admin` once (see HOSTING.md).
CMD ["/app/multidirectory"]
