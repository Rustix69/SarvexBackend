# 07 — Gateways: `gw-rest` and `gw-ws`

The two services that sit at the edge between clients and internal gRPC. Stateless. Horizontally scalable.

---

## 1. `gw-rest` (REST/JSON gateway)

### 1.1 Responsibility

Public HTTPS API. Translates JSON ↔ protobuf. Handles auth, rate limiting, idempotency, and request shape validation. Owns no domain logic — every endpoint is a thin proxy to a backend service.

### 1.2 Technology

- **Language:** Go
- **HTTP framework:** `chi` (lightweight, std-lib compatible)
- **Validation:** `go-playground/validator` for input shape; `protovalidate` for cross-cutting rules
- **Auth:** JWT (HS256 in demo, RS256 in production)
- **Rate limiting:** `golang.org/x/time/rate` in-process (demo); Redis token bucket (production)
- **Idempotency:** in-process LRU (demo); Redis with 24h TTL (production)

### 1.3 Middleware chain (in order)

```
request
  │
  ▼
1. RequestID middleware    → assigns trace_id, propagates to gRPC
2. Logging middleware      → structured JSON log per request
3. Recovery middleware     → recovers panics, returns 500
4. CORS middleware         → demo: permissive; prod: locked to known origins
5. Rate-limit middleware   → per-IP for unauth, per-user for auth
6. Auth middleware         → decodes JWT, sets user_id in context
7. Idempotency middleware  → on mutating endpoints, dedupes on Idempotency-Key
8. Handler                 → translates JSON → proto, calls gRPC, translates back
  │
  ▼
response
```

### 1.4 Auth

**Demo:**
- `POST /v1/auth/login` accepts `{email, password}`, returns JWT signed with `JWT_SECRET` env var.
- JWT claims: `sub` (user_id), `iat`, `exp` (24h), `kyc_tier`, `is_admin`, `is_mm`.
- Every authenticated endpoint requires `Authorization: Bearer <jwt>`.

**Production additions (not built in demo):**
- TOTP MFA on login
- API key auth via Ed25519 request signing (HTTP-Signatures style)
- Refresh tokens with rotation
- Session revocation list in Redis

```go
// pkg/auth/jwt.go
func (a *Authenticator) Verify(token string) (*Claims, error) {
    parsed, err := jwt.Parse(token, func(t *jwt.Token) (any, error) {
        if t.Method != jwt.SigningMethodHS256 {  // RS256 in prod
            return nil, fmt.Errorf("unexpected signing method")
        }
        return a.secret, nil
    })
    if err != nil { return nil, err }
    if !parsed.Valid { return nil, ErrInvalidToken }
    claims := parsed.Claims.(jwt.MapClaims)
    return &Claims{
        UserID:  claims["sub"].(string),
        KYCTier: int(claims["kyc_tier"].(float64)),
        IsAdmin: claims["is_admin"].(bool),
        IsMM:    claims["is_mm"].(bool),
    }, nil
}
```

### 1.5 Rate limiting

**Demo:** token bucket per user_id (or per IP if unauthenticated). Defaults:
- Unauth: 30 req/s, burst 60
- Authenticated retail: 50 req/s, burst 100
- Authenticated MM: 500 req/s, burst 1000

**Production:** same logic but in Redis (so it survives pod restarts and works across replicas). Plus a global rate limit per endpoint to protect downstream.

Configured per-endpoint in code:
```go
r.With(rateLimit(50, 100)).Post("/v1/orders", h.submitOrder)
r.With(rateLimit(100, 200)).Delete("/v1/orders/{order_id}", h.cancelOrder)
```

### 1.6 Idempotency

Every mutating endpoint accepts an `Idempotency-Key` header (max 64 chars). Endpoints that can move funds or create exchange state, especially `POST /v1/orders`, should require it. The middleware:
1. Hashes the key + endpoint + user_id.
2. Looks up in cache. If present, returns cached response.
3. Otherwise, captures the response (status + body) on the way out and stores with 24h TTL.

```go
func IdempotencyMiddleware(cache IdemCache) func(http.Handler) http.Handler {
    return func(next http.Handler) http.Handler {
        return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
            key := r.Header.Get("Idempotency-Key")
            if r.Method == "GET" {
                next.ServeHTTP(w, r); return
            }
            if requiresIdempotency(r) && key == "" {
                writeError(w, 400, "IDEMPOTENCY_KEY_REQUIRED"); return
            }
            if key == "" {
                next.ServeHTTP(w, r); return
            }
            cacheKey := fmt.Sprintf("%s:%s:%s",
                userIDFromContext(r.Context()), r.URL.Path, key)
            if cached, ok := cache.Get(cacheKey); ok {
                w.Header().Set("X-Idempotent-Replay", "true")
                w.WriteHeader(cached.Status)
                w.Write(cached.Body); return
            }
            rec := httptest.NewRecorder()
            next.ServeHTTP(rec, r)
            if rec.Code < 500 {  // don't cache server errors
                cache.Set(cacheKey, &CachedResp{Status: rec.Code, Body: rec.Body.Bytes()}, 24*time.Hour)
            }
            for k, v := range rec.Header() { w.Header()[k] = v }
            w.WriteHeader(rec.Code)
            w.Write(rec.Body.Bytes())
        })
    }
}
```

Order idempotency is not only the gateway cache. The router still enforces `(user_id, client_order_id)` and me-core still dedupes by `order_id`.

### 1.7 JSON ↔ proto conversion

Use `protojson` from `google.golang.org/protobuf/encoding/protojson` with these options:
- `EmitUnpopulated: true` (so zero values appear in responses for predictability)
- `UseProtoNames: false` (use camelCase in JSON; proto field `client_order_id` → JSON `clientOrderId`)
- `Multiline: false` (compact JSON over the wire)

For input, accept both snake_case and camelCase (use `AllowPartial: true, DiscardUnknown: true`).

### 1.8 Error response shape

Consistent error envelope:
```json
{
  "error": {
    "code": "INSUFFICIENT_FUNDS",
    "message": "Account balance 42 USDC insufficient for required hold 62 USDC.",
    "details": { "required_micro_usdc": 62000000, "available_micro_usdc": 42000000 },
    "trace_id": "01H..."
  }
}
```

Mapping from gRPC status codes:
- `InvalidArgument` → 400
- `Unauthenticated` → 401
- `PermissionDenied` → 403
- `NotFound` → 404
- `AlreadyExists` → 409
- `FailedPrecondition` → 422 (used for risk rejects, balance fails)
- `ResourceExhausted` → 429 (rate limit) or 503 (queue full)
- `DeadlineExceeded` → 504
- `Unavailable` → 503
- Other → 500

### 1.9 File layout

```
services/gw-rest/
├── cmd/server/main.go
├── internal/
│   ├── api/                   # one handler file per resource group
│   │   ├── orders.go
│   │   ├── markets.go
│   │   ├── account.go
│   │   ├── auth.go
│   │   ├── admin.go
│   │   └── ws_redirect.go     # /v1/stream → 101 Switching Protocols (delegates to gw-ws via header in prod)
│   ├── middleware/
│   ├── codec/                 # JSON ↔ proto helpers
│   ├── auth/                  # JWT verify
│   ├── ratelimit/
│   ├── idem/
│   └── grpcclients/           # cached gRPC clients to all backend services
├── go.mod
└── Dockerfile
```

### 1.10 Endpoint → backend mapping

| HTTP endpoint | Backend gRPC |
|---|---|
| `POST /v1/auth/login` | (internal: users table lookup + bcrypt + JWT issue) |
| `POST /v1/orders` | `OrderRouter.SubmitOrder` |
| `DELETE /v1/orders/{order_id}` | `OrderRouter.CancelOrder` |
| `POST /v1/orders/{order_id}/amend` | `OrderRouter.AmendOrder` |
| `GET /v1/orders` | `OrderRouter.ListOrders` |
| `GET /v1/orders/{order_id}` | `OrderRouter.GetOrder` |
| `GET /v1/markets` | `RefData.ListContracts` |
| `GET /v1/markets/{ticker}` | `RefData.GetContract` |
| `GET /v1/markets/{ticker}/orderbook` | `MatchingEngine.GetBookSnapshot` |
| `GET /v1/positions` | `Position.ListPositions` |
| `GET /v1/positions/{ticker}` | `Position.GetPosition` |
| `GET /v1/account/balance` | `Ledger.GetBalance` |
| `GET /v1/account/history` | `Ledger.GetAccountHistory` |
| `POST /v1/admin/deposits/credit` | `Ledger.AdminCreditDeposit` (gated by `is_admin`) |
| `POST /v1/admin/contracts/{ticker}/resolve` | `Oracle.AdminForceResolution` (gated by `is_admin`) |

---

## 2. `gw-ws` (WebSocket gateway)

### 2.1 Responsibility

Real-time push to clients. Subscribes to NATS subjects and fans them out to interested WebSocket connections. Stateless per-pod (connection state is in-memory but a disconnect just means reconnect).

### 2.2 Technology

- **Language:** Go
- **WebSocket library:** `nhooyr.io/websocket` (modern, context-aware, no abandoned `gorilla/websocket` baggage)
- **Codec:** JSON for client-facing; protobuf internally
- **NATS:** subject-based subscriptions, in-process routing

### 2.3 Connection lifecycle

```
Client opens WSS
  → handshake (with optional ?token=<jwt> for auth on first connect)
  → server sends: {"type": "welcome", "msg": {"sid_namespace": "<random>", "server_time": "..."}}
  → client sends: {"id": 1, "cmd": "subscribe", "params": {...}}
  → server responds: {"type": "subscribed", "id": 1, "sid": 42}
  → server pushes events tagged with sid: {"type": "...", "sid": 42, "seq": N, "msg": {...}}
  → heartbeat every 15s (Ping/Pong)
  → on client disconnect: tear down all sids
```

### 2.4 Subscription model

A single connection can hold many `sid`s (subscription IDs). Each sid binds:
- A channel name (e.g., `orderbook_delta`)
- A scope (e.g., a specific ticker, or "all my orders")

Server maintains:
```go
type Connection struct {
    conn     *websocket.Conn
    userID   string  // empty if unauthenticated
    sids     map[uint32]*Subscription  // sid → subscription
    nextSID  uint32
    sendCh   chan []byte    // outbound message queue
    closed   atomic.Bool
}

type Subscription struct {
    sid       uint32
    channel   string
    scope     ScopeKey      // e.g. {"ticker": "RBI-JUN26-CUT25"}
    natsSubs  []*nats.Subscription  // subjects this sid pulls from
    seq       uint64                 // monotonic per-sid
}
```

### 2.5 Channel → NATS subject mapping

| Channel | NATS subject | Auth? |
|---|---|---|
| `orderbook_snapshot` | (special: gRPC pull from me-core + buffered deltas) | no |
| `orderbook_delta` | `md.book.<ticker>` | no |
| `trades` | `md.trade.<ticker>` | no |
| `ticker` | `md.ticker.<ticker>` | no |
| `market_lifecycle` | `md.lifecycle.<ticker>` | no |
| `user_orders` | `exec.user.<user_id>` (sanitized by order-router) | yes |
| `user_fills` | `exec.fills.user.<user_id>` (sanitized by order-router) | yes |
| `user_balance` | `ledger.balance.user.<user_id>` | yes |

Internal services consume `exec.events` and `exec.fills.<ticker>`. Gateways should not expose raw internal fill events because they include both counterparties.

### 2.6 Snapshot + delta protocol

On subscribing to `orderbook_delta` for a ticker:
1. Server subscribes to `md.book.<ticker>` first and buffers deltas for this subscription.
2. Server issues `MatchingEngine.GetBookSnapshot(ticker, depth=20)`.
3. Server sends `{"type":"orderbook_snapshot","sid":N,"seq":snapshot.seq,"msg":...}`.
4. Server discards buffered deltas with `seq <= snapshot.seq` and replays buffered deltas with `seq > snapshot.seq` in order.
5. Server then switches the subscription to live forwarding.

Client uses `seq` to detect gaps. If a gap is detected, client sends:
```json
{"id": 99, "cmd": "resync", "params": {"sid": N}}
```
Server repeats the subscribe-buffer-snapshot-replay flow. Production additionally keeps a 1-second rolling cache of recent deltas per ticker so small gaps can recover without a full gRPC snapshot. Demo still does full snapshot on every detected gap, but it must buffer during snapshot creation to avoid losing deltas.

### 2.7 Backpressure handling

Each connection has a bounded `sendCh` (default 256 messages). If full:
- For market-data channels (lossy is OK): drop oldest unsent message, add new.
- For private channels (lossless required): close the connection with code 1008 ("policy violation"). Client must reconnect and re-snapshot.

```go
func (c *Connection) enqueue(msg []byte, lossy bool) {
    select {
    case c.sendCh <- msg: // ok
    default:
        if lossy {
            select { case <-c.sendCh: default: }  // drop one
            select { case c.sendCh <- msg: default: c.closeWithCode(1008) }
        } else {
            c.closeWithCode(1008)
        }
    }
}
```

### 2.8 Auth flow

Two options for clients:
1. **Query param at handshake:** `wss://api/v1/stream?token=<jwt>` — sets userID at connect time.
2. **Auth message:** client sends `{"cmd": "auth", "params": {"jwt": "..."}}` after connect.

Both work. Option 2 lets a client share one WSS connection across login/logout cycles in a single page.

Once authenticated, the userID is bound to the connection for its lifetime; clients cannot switch identities mid-connection (they reconnect).

### 2.9 Per-user routing

Internal services (order-router, ledger-svc) publish to per-user subjects:
- `exec.user.u_42` — sanitized order events for user u_42
- `exec.fills.user.u_42` — sanitized fills for user u_42
- `ledger.balance.user.u_42` — balance updates

`gw-ws` subscribes to these on demand (when a user first asks for their feed) and routes to the matching connection. A user with multiple browser tabs has multiple connections; gw-ws maintains an `userID → []connections` map and fans out.

```go
type UserFanout struct {
    mu     sync.RWMutex
    conns  map[string][]*Connection
}

func (uf *UserFanout) onNATSMessage(subj string, data []byte) {
    userID := extractUserID(subj)
    uf.mu.RLock()
    conns := uf.conns[userID]
    uf.mu.RUnlock()
    for _, c := range conns {
        c.enqueue(data, false)  // private = lossless
    }
}
```

### 2.10 Demo simplifications

- One pod. NATS subscription is direct (no multi-pod routing).
- No gap recovery cache; client resyncs from snapshot on gap.
- No persistent session ID — reconnect = new connection; client resubscribes and receives a fresh snapshot.

### 2.11 Production additions

- Multi-pod: each pod subscribes to NATS independently; doesn't matter if multiple pods see the same message (each only fans out to connections it owns).
- Session resumption: persistent session ID with 30s grace period for reconnect; messages buffered server-side during grace.
- Gap recovery cache: per-ticker ring buffer of last 1024 deltas.
- mTLS upstream to NATS.

### 2.12 File layout

```
services/gw-ws/
├── cmd/server/main.go
├── internal/
│   ├── server/             # WebSocket handler, connection mgmt
│   ├── subscriptions/      # sid → NATS routing
│   ├── fanout/             # per-user routing
│   ├── auth/               # JWT verify (shared with gw-rest via pkg/auth)
│   ├── codec/              # proto ↔ JSON
│   └── grpcclients/        # me-core client (for snapshots)
├── go.mod
└── Dockerfile
```

---

## 3. Common: `pkg/auth`

Both gateways use the same JWT helper:

```go
// pkg/auth/jwt.go
type Claims struct {
    UserID  string
    KYCTier int
    IsAdmin bool
    IsMM    bool
}

type Authenticator interface {
    Verify(tokenString string) (*Claims, error)
    Issue(claims *Claims, ttl time.Duration) (string, error)
}

func NewHS256(secret []byte) Authenticator { ... }
func NewRS256(privKey, pubKey []byte) Authenticator { ... }  // production
```

Configuration:
- Demo: `JWT_SECRET` env var (HS256)
- Production: `JWT_PRIVATE_KEY_PATH` + `JWT_PUBLIC_KEY_PATH` (RS256); private key mounted from KMS-decrypted secret

---

## 4. Operational concerns

### 4.1 Logging

Every request emits one structured JSON log line at completion:
```json
{
  "ts": "2026-05-19T10:21:13.412Z",
  "level": "info",
  "service": "gw-rest",
  "endpoint": "POST /v1/orders",
  "user_id": "u_42",
  "status": 201,
  "duration_ms": 23,
  "trace_id": "01H...",
  "client_order_id": "ord_abc",
  "result": "accepted"
}
```

Errors include `error.code`, `error.message`, and (in production) a stack reference.

### 4.2 Tracing

`pkg/tracing/otel.go` initializes OpenTelemetry. Every gRPC outbound call propagates the trace context. In demo, the exporter is the no-op exporter; in production, OTLP to Tempo.

### 4.3 Metrics

Per-endpoint:
- `gw_rest_request_duration_seconds` (histogram, labels: endpoint, status)
- `gw_rest_requests_total` (counter)
- `gw_ws_connections` (gauge)
- `gw_ws_subscriptions` (gauge, labels: channel)
- `gw_ws_messages_sent_total` (counter, labels: channel)
- `gw_ws_messages_dropped_total` (counter, labels: channel, reason)

---

## 5. Demo configuration

`docker-compose.yml` exposes:
- `gw-rest`: localhost:8080
- `gw-ws`: localhost:8081
- frontend: localhost:3000

Frontend's `lib/api.ts` and `lib/ws.ts` point to these. In production, both are behind CloudFront + ALB at `api.sarvex.com`.

---

## 6. What's deliberately not in the gateways

- **No business logic.** If a check belongs in risk or order-router, it doesn't go here.
- **No caching of business data.** Refdata caching is in `order-router` and frontend; gateway is stateless.
- **No persistence.** Gateway has no database. (Idempotency cache is ephemeral; loss = retry charges, not a problem.)
- **No batching.** Each REST request is one operation. (LP-grade batch endpoints are Phase 3.)
