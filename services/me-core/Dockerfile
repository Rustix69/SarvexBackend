FROM alpine:3.20 AS builder
RUN apk add --no-cache build-base cmake protobuf-dev
WORKDIR /app
COPY liquibook ./liquibook
COPY services/me-core ./services/me-core
WORKDIR /app/services/me-core
RUN cmake -S . -B build && cmake --build build --target me-core

FROM alpine:3.20
RUN apk add --no-cache libstdc++
COPY --from=builder /app/services/me-core/build/me-core /usr/local/bin/me-core
ENTRYPOINT ["/usr/local/bin/me-core"]
