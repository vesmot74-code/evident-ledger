# Build stage
FROM rust:1.88-slim AS builder
WORKDIR /app

RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

COPY . .
ENV SQLX_OFFLINE=true
    RUN cargo build --release --bin evident-ledger

# Runtime stage
FROM debian:bookworm-slim
WORKDIR /app

RUN apt-get update && apt-get install -y ca-certificates libssl3 && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/evident-ledger /app/evident-ledger

EXPOSE 3000
CMD ["/app/evident-ledger"]
