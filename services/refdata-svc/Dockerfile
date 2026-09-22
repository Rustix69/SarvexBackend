FROM rust:1.88-bookworm AS builder

RUN apt-get update \
    && apt-get install -y --no-install-recommends protobuf-compiler \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY services ./services
COPY proto ./proto

RUN cargo build --manifest-path services/Cargo.toml --release -p refdata-svc

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/services/target/release/refdata-svc /usr/local/bin/refdata-svc
EXPOSE 50051 8080
ENTRYPOINT ["/usr/local/bin/refdata-svc"]
