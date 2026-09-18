FROM golang:1.24-alpine AS builder
WORKDIR /app
COPY go.mod go.sum ./
RUN go mod download
COPY . .
RUN CGO_ENABLED=0 GOOS=linux GOARCH=amd64 go build -o /out/svc-server ./cmd/svc-server

FROM alpine:3.20
RUN apk add --no-cache ca-certificates wget
COPY --from=builder /out/svc-server /svc-server
ENTRYPOINT ["/svc-server"]
