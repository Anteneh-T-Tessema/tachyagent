# Multi-stage build for minimal image size
FROM rust:1.82-slim AS builder

WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/

# Build release binary
RUN cargo build -p tachy-cli --release

# Runtime image — minimal, no Rust toolchain
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates git curl \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/tachy-cli /usr/local/bin/tachy

# Create non-root user
RUN useradd -m -s /bin/bash tachy
USER tachy
WORKDIR /home/tachy/workspace

EXPOSE 7777

ENTRYPOINT ["tachy"]
CMD ["serve", "0.0.0.0:7777"]
