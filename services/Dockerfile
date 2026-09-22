FROM rust:1.88-bookworm AS builder

RUN apt-get update \
    && apt-get install -y --no-install-recommends protobuf-compiler \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY services ./services
COPY proto ./proto
ARG SERVICE
RUN test -n "$SERVICE"
RUN cargo build --manifest-path services/Cargo.toml --release -p "$SERVICE"

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
ARG SERVICE
RUN test -n "$SERVICE"
COPY --from=builder /app/services/target/release/${SERVICE} /usr/local/bin/service
ENTRYPOINT ["/usr/local/bin/service"]
