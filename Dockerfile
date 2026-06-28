# ── Build stage ───────────────────────────────────────────────────────────────
FROM rust:1.88-slim AS builder

WORKDIR /app

# native-tls build deps
RUN apt-get update && \
    apt-get install -y pkg-config libssl-dev && \
    rm -rf /var/lib/apt/lists/*

# Cache dependencies before copying source.
# A dummy main lets cargo fetch and compile all deps in a cacheable layer.
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo 'fn main() {}' > src/main.rs && \
    cargo build --release && \
    rm -rf src

# Build the real binary
COPY src ./src
COPY migrations ./migrations
RUN touch src/main.rs && cargo build --release

# ── Runtime stage ─────────────────────────────────────────────────────────────
FROM debian:bookworm-slim AS runtime

WORKDIR /app

# ca-certificates for TLS to Neon; libssl3 for native-tls at runtime
RUN apt-get update && \
    apt-get install -y ca-certificates libssl3 && \
    rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/the-boys-api ./the-boys-api

EXPOSE 3000

CMD ["./the-boys-api"]
