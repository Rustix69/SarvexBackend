FROM debian:bookworm AS builder

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
       ca-certificates cmake g++ make pkg-config protobuf-compiler protobuf-compiler-grpc \
       libprotobuf-dev libgrpc++-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /src
COPY backend-old/liquibook ./backend-old/liquibook
COPY backend-old/proto ./backend-old/proto
COPY services/me-core ./services/me-core

RUN cmake -S services/me-core -B build -DCMAKE_BUILD_TYPE=Release \
    && cmake --build build --parallel

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates libgrpc++1.51 libprotobuf32 \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /src/build/me-core /usr/local/bin/me-core
EXPOSE 50054
ENV ME_CORE_LISTEN_ADDR=0.0.0.0:50054
ENTRYPOINT ["/usr/local/bin/me-core"]
