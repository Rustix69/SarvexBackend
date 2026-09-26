# Sarvex Trading API

This document describes the public Sarvex API currently exposed by `gw-rest`
and `gw-ws`. It is the client-facing contract for applications, bots, and
manual API testing.

The API is JSON over HTTPS. Production base URL:

```text
https://api.sarvaex.com
```

Local Docker base URL:

```text
http://localhost:18080
```

All timestamps are RFC 3339 strings. All counts, sequence numbers, prices,
and money values are integers. Never use floating point for order prices or
money.

## Conventions

### Authentication

Authenticated requests use:

```http
Authorization: Bearer <token>
```

`AUTH_MODE=demo` is used for local/demo deployments. The login response token
is an identity token in the form `demo.<base64url(user_id)>`; it is not secure
authentication and must not be used for a real public deployment.

`AUTH_MODE=jwt` is the production code path currently implemented. Login
requires the configured `AUTH_LOGIN_SECRET`, and the gateway issues an HS256
JWT with the configured issuer, audience, and expiration. The JWT secret must
be at least 32 bytes.

### Idempotency

Every mutating request requires a unique `Idempotency-Key` header:

```http
Idempotency-Key: order-20260925-000001
```

The key is scoped to the authenticated user, HTTP method, and request path.
Retrying the same request with the same key returns the stored response.
Reusing a key with a different request body returns `409 IDEMPOTENCY_KEY_REUSED`.
If the first attempt was durably accepted but its result is not known yet,
retries return `409 OPERATION_IN_PROGRESS`; reuse the same key while the
gateway reconciles the original command.

Use a new key for every logical order submission, cancellation, or demo credit.

### Error envelope

Errors use one stable shape:

```json
{
  "error": {
    "code": "INVALID_ARGUMENT",
    "message": "count must be positive"
  }
}
```

Common HTTP statuses:

| Status | Meaning |
|---:|---|
| `400` | Invalid request, enum, price, count, or contract state |
| `401` | Missing or invalid Bearer token |
| `403` | Authenticated but not permitted |
| `404` | Contract, order, or position was not found |
| `409` | Idempotency conflict or state conflict |
| `504` | Upstream deadline exceeded where applicable |
| `502` | Internal service or matching-engine failure |
| `503` | Gateway, database, or matching-engine unavailable |

## Authentication

### Login

```http
POST /v1/auth/login
Content-Type: application/json
```

Request:

```json
{
  "user_id": "demo-user-1",
  "password": "<AUTH_LOGIN_SECRET>"
}
```

`password` is optional in demo mode and required in JWT mode. The server does
not create users; `user_id` identifies the existing demo/trading account.

Response `200`:

```json
{
  "token": "demo.ZGVtby11c2VyLTE",
  "token_type": "Bearer",
  "user_id": "demo-user-1"
}
```

## Service Health

These endpoints do not require authentication:

```http
GET /healthz
GET /readyz
GET /metrics
GET /v1/health/overview
```

`/healthz` and `/readyz` return a small service status object. The overview
returns `summary.running`, `summary.total`, `summary.not_running`, and an
`items` array containing service name, kind, status, latency, and probe detail.
The Prometheus endpoint is plain text.

## Public Market Data

### List markets

```http
GET /v1/markets?state=OPEN&series_ticker=XLSX-STANDARD&limit=50&cursor=<cursor>
```

All query parameters are optional:

| Parameter | Type | Description |
|---|---|---|
| `state` | string | `DRAFT`, `LISTED`, `OPEN`, `HALTED`, `CLOSED`, `RESOLVING`, `SETTLED`, or `CANCELLED` |
| `series_ticker` | string | Restrict to one series |
| `limit` | integer | `1` to `500`; default `50` |
| `cursor` | string | Cursor returned by the previous response |

Response `200`:

```json
{
  "contracts": [
    {
      "ticker": "SX-FEDDEC-26OCT-H25",
      "event_ticker": "XEV-STD-ECO-01",
      "series_ticker": "XLSX-STANDARD",
      "kind": 1,
      "question": "Will the FOMC raise the federal funds target range by exactly 25 bp at its 27-28 Oct 2026 meeting?",
      "underlying": "",
      "tick_size": 1,
      "min_price_ticks": 1,
      "max_price_ticks": 99,
      "lower_bound_ticks": 1,
      "upper_bound_ticks": 99,
      "multiplier_micro_usdc": 0,
      "divider": 100,
      "multiplier_micro_per_display_unit": 0,
      "tick_value_micro": 0,
      "max_order_size": 100000,
      "position_limit_per_user": 250000,
      "state": 3,
      "listed_at": "2026-01-01T00:00:00Z",
      "open_at": "2026-01-01T00:00:00Z",
      "close_at": "2026-10-28T23:59:00Z",
      "expected_resolution_at": "2026-10-28T23:59:00Z",
      "settlement_source": "Source agency and Sarvex oracle policy",
      "oracle_policy": "ADMIN",
      "settlement_rule": {},
      "close_global_seq": 0
    }
  ],
  "next_cursor": ""
}
```

`kind` and `state` are currently returned as protobuf enum integers:

| `kind` | Meaning |
|---:|---|
| `1` | Binary |
| `2` | Scalar/futures |

| `state` | Meaning |
|---:|---|
| `1` | Draft |
| `2` | Listed |
| `3` | Open |
| `4` | Closed |
| `5` | Resolving |
| `6` | Settled |
| `7` | Cancelled |
| `8` | Halted |

### Get one market

```http
GET /v1/markets/{ticker}
```

Response `200` is one contract object with the same fields as an item in the
list response.

### Get an order-book snapshot

```http
GET /v1/markets/{ticker}/orderbook?depth=20
```

`depth` is clamped to `1` through `100`; the default is `20`.

Response `200`:

```json
{
  "ticker": "SX-FEDDEC-26OCT-H25",
  "seq": 1842,
  "ts": "2026-09-25T09:10:00Z",
  "bids": [
    { "price_ticks": 49, "total_qty": 120, "order_count": 8 }
  ],
  "asks": [
    { "price_ticks": 51, "total_qty": 96, "order_count": 6 }
  ]
}
```

For binary contracts, `price_ticks` normally represents cents, so `49` is
`$0.49` and the complementary NO price is `$0.51`. For futures/scalar
contracts, use the contract's `divider`, `tick_value_micro`, and multiplier
metadata instead of assuming cents.

### List fills for a market

```http
GET /v1/markets/{ticker}/fills?from_global_seq=0&to_global_seq=0&limit=100&cursor=<cursor>
```

Response `200`:

```json
{
  "fills": [
    {
      "fill_id": "fill-123",
      "ticker": "SX-FEDDEC-26OCT-H25",
      "global_seq": 8201,
      "contract_seq": 421,
      "maker_order_id": "ord-maker",
      "taker_order_id": "ord-taker",
      "maker_user_id": "user-maker",
      "taker_user_id": "user-taker",
      "maker_hold_id": "hold-maker",
      "taker_hold_id": "hold-taker",
      "maker_side": "YES",
      "maker_action": "BUY",
      "taker_side": "NO",
      "taker_action": "BUY",
      "price_ticks": 51,
      "count": 10,
      "aggressor_side": "NO",
      "maker_fee_micro_usdc": 0,
      "taker_fee_micro_usdc": 0,
      "ts": "2026-09-25T09:10:00Z"
    }
  ],
  "next_cursor": ""
}
```

This endpoint is public market data. A client should not use it as a private
order-status feed; use the authenticated order endpoints or private WebSocket
channel for user-specific execution updates.

### Get open interest

```http
GET /v1/markets/{ticker}/open-interest
```

Response `200`:

```json
{
  "ticker": "SXF-FFUB-26OCT",
  "total_open_long": 120,
  "total_open_short": 120,
  "as_of_global_seq": 8201,
  "as_of": "2026-09-25T09:10:00Z"
}
```

## Trading

### Submit an order

```http
POST /v1/orders
Authorization: Bearer <token>
Idempotency-Key: order-20260925-000001
Content-Type: application/json
```

Request:

```json
{
  "client_order_id": "client-order-000001",
  "ticker": "SX-FEDDEC-26OCT-H25",
  "side": "YES",
  "action": "BUY",
  "order_type": "LIMIT",
  "price_ticks": 51,
  "count": 10,
  "tif": "GTC",
  "post_only": false,
  "reduce_only": false,
  "stp": "UNSPECIFIED",
  "expires_at": null
}
```

The accepted request fields are:

| Field | Type | Required | Description |
|---|---|---:|---|
| `client_order_id` | string | yes | Client-generated id for reconciliation |
| `ticker` | string | yes | Open contract ticker |
| `side` | string | yes | Binary: `YES`/`NO`; futures: `LONG`/`SHORT` |
| `action` | string | yes | `BUY` or `SELL` |
| `order_type` | string | no | `LIMIT` or `MARKET`; `type` is accepted as an alias |
| `price_ticks` | integer | limit | Positive, tick-aligned limit price |
| `count` | integer | yes | Positive contract quantity |
| `tif` | string | no | `GTC`, `IOC`, or `FOK`; default is `GTC` |
| `post_only` | boolean | no | Reject/cancel if the order would immediately take liquidity |
| `reduce_only` | boolean | no | Restrict the order to reducing an existing position |
| `stp` | string | no | `TAKER_AT_CROSS`, `MAKER`, or `UNSPECIFIED` |
| `expires_at` | timestamp/null | no | RFC 3339 expiry for supported order lifecycles |

For a market order, send `order_type: "MARKET"`. The gateway reads the live
opposite-side quote, derives a bounded protection price, and submits the order
as IOC. `price_ticks` may be `0` for this request because the gateway derives
the protection price. A market order is rejected with
`MARKET_NOT_AVAILABLE` when no live opposite-side quote exists.

Binary order example:

```json
{
  "client_order_id": "bin-000001",
  "ticker": "SX-FEDDEC-26OCT-H25",
  "side": "YES",
  "action": "BUY",
  "order_type": "LIMIT",
  "price_ticks": 51,
  "count": 10,
  "tif": "GTC"
}
```

Futures/scalar order example:

```json
{
  "client_order_id": "future-000001",
  "ticker": "SXF-USCPIYOY-26SEP",
  "side": "LONG",
  "action": "BUY",
  "order_type": "LIMIT",
  "price_ticks": 430,
  "count": 2,
  "tif": "GTC"
}
```

Response `200`:

```json
{
  "order": {
    "order_id": "ord-123",
    "client_order_id": "client-order-000001",
    "user_id": "demo-user-1",
    "ticker": "SX-FEDDEC-26OCT-H25",
    "side": "YES",
    "action": "BUY",
    "price_ticks": 51,
    "count": 10,
    "filled_count": 10,
    "remaining_count": 0,
    "tif": "GTC",
    "post_only": false,
    "reduce_only": false,
    "stp": "UNSPECIFIED",
    "status": "FILLED",
    "created_at": "2026-09-25T09:10:00Z",
    "updated_at": "2026-09-25T09:10:00Z",
    "expires_at": null,
    "hold_id": "hold-123",
    "avg_fill_price_ticks": 51
  },
  "fills": [
    {
      "fill_id": "fill-123",
      "order_id": "ord-123",
      "ticker": "SX-FEDDEC-26OCT-H25",
      "price_ticks": 51,
      "count": 10,
      "aggressor_side": "YES",
      "fee_micro_usdc": 0,
      "ts": "2026-09-25T09:10:00Z",
      "seq": 8201
    }
  ],
  "reject_code": "",
  "reject_reason": ""
}
```

An accepted request can still return an order with `OPEN`, `PARTIAL`,
`CANCELLED`, or `REJECTED` status depending on liquidity, TIF, risk, and
matching results. Always inspect `order.status`, `filled_count`, and
`remaining_count`; do not infer execution from HTTP `200` alone.

### List the authenticated user's orders

```http
GET /v1/orders?ticker=SX-FEDDEC-26OCT-H25&status=OPEN&limit=100&cursor=<cursor>
Authorization: Bearer <token>
```

`status` accepts `PENDING`, `OPEN`, `PARTIAL`, `FILLED`, `CANCELLED`,
`REJECTED`, or `EXPIRED`.

Response `200`:

```json
{
  "orders": [
    {
      "order_id": "ord-123",
      "client_order_id": "client-order-000001",
      "user_id": "demo-user-1",
      "ticker": "SX-FEDDEC-26OCT-H25",
      "side": "YES",
      "action": "BUY",
      "price_ticks": 51,
      "count": 10,
      "filled_count": 4,
      "remaining_count": 6,
      "cancelled_count": 0,
      "expired_count": 0,
      "tif": "GTC",
      "post_only": false,
      "reduce_only": false,
      "stp": "UNSPECIFIED",
      "status": "PARTIAL",
      "created_at": "2026-09-25T09:10:00Z",
      "updated_at": "2026-09-25T09:10:01Z",
      "expires_at": null,
      "hold_id": "hold-123",
      "avg_fill_price_ticks": 51
    }
  ],
  "next_cursor": ""
}
```

### Get one order

```http
GET /v1/orders/{order_id}
Authorization: Bearer <token>
```

Response `200` is one order object. Users can only retrieve their own orders.

### Cancel an order

```http
POST /v1/orders/{order_id}/cancel
Authorization: Bearer <token>
Idempotency-Key: cancel-20260925-000001
Content-Type: application/json
```

The request body is currently ignored and may be `{}`:

```json
{}
```

Response `200`:

```json
{
  "order": {
    "order_id": "ord-123",
    "status": "CANCELLED",
    "count": 10,
    "filled_count": 4,
    "remaining_count": 0,
    "cancelled_count": 6,
    "expired_count": 0
  },
  "reject_code": "",
  "reject_reason": ""
}
```

Cancellation is idempotent at the gateway using the same key. A filled or
already terminal order may return a rejection/state error from the router.

### Amend orders

There is currently no public REST amend route. The internal protobuf contract
contains `AmendOrder`, but clients must not call it over REST until a gateway
route and idempotency contract are added.

## Account and Portfolio

### Get balance

```http
GET /v1/account/balance
Authorization: Bearer <token>
```

Response `200`:

```json
{
  "user_id": "demo-user-1",
  "cash_micro_usdc": 10000000000,
  "held_micro_usdc": 5100000,
  "total_micro_usdc": 10005100000
}
```

Values are micro-USDC. Divide by `1_000_000` for display only.

### Get account ledger history

```http
GET /v1/account/history?limit=100&cursor=<cursor>
Authorization: Bearer <token>
```

Response `200`:

```json
{
  "entries": [
    {
      "tx_id": "tx-123",
      "account_code": "USER:demo-user-1:CASH",
      "direction": "CR",
      "amount_micro_usdc": 10000000000,
      "running_balance_micro_usdc": 10000000000,
      "reason_code": "DEMO_DEPOSIT",
      "posted_at": "2026-09-25T09:00:00Z",
      "memo": "demo funding"
    }
  ],
  "next_cursor": ""
}
```

### Credit demo funds

This endpoint is for demo/devnet accounts only.

```http
POST /v1/demo/deposits/credit
Authorization: Bearer <token>
Idempotency-Key: deposit-20260925-000001
Content-Type: application/json
```

Request using micro-USDC:

```json
{
  "amount_micro_usdc": 10000000000,
  "note": "demo funding"
}
```

Or using whole USDC:

```json
{
  "amount_usdc": 10000,
  "note": "demo funding"
}
```

Response `200`:

```json
{
  "ok": true,
  "balance": {
    "user_id": "demo-user-1",
    "cash_micro_usdc": 10000000000,
    "held_micro_usdc": 0,
    "total_micro_usdc": 10000000000
  }
}
```

### List positions

```http
GET /v1/positions?include_closed=false&limit=100&cursor=<cursor>
Authorization: Bearer <token>
```

`include_closed=false` excludes retained zero-quantity positions.
`include_closed=true` includes them when the account needs realized P&L or
historical records. `limit` is clamped to `1` through `500`; `next_cursor`
is a ticker cursor that preserves this filter.

Response `200`:

```json
{
  "positions": [
    {
      "user_id": "demo-user-1",
      "ticker": "SX-FEDDEC-26OCT-H25",
      "net_qty": 10,
      "avg_cost_micro_usdc": 510000,
      "realized_pnl_micro_usdc": 0,
      "unrealized_pnl_micro_usdc": 0,
      "updated_at": "2026-09-25T09:10:00Z",
      "last_global_seq": 8201
    }
  ],
  "next_cursor": ""
}
```

### Get one position

```http
GET /v1/positions/{ticker}
Authorization: Bearer <token>
```

Response `200` is one position object with the fields shown above.

## WebSocket API

The WebSocket endpoint is:

```text
wss://api.sarvaex.com/ws
```

Local:

```text
ws://localhost:18082/ws
```

Send the Bearer token as an HTTP upgrade header when subscribing to private
fills:

```http
Authorization: Bearer <token>
```

On connection the server sends:

```json
{ "type": "connected", "service": "gw-ws" }
```

### Subscribe to market data

Client message:

```json
{
  "op": "subscribe",
  "channel": "market",
  "ticker": "SX-FEDDEC-26OCT-H25"
}
```

Acknowledgement:

```json
{
  "type": "subscribed",
  "channel": "market",
  "ticker": "SX-FEDDEC-26OCT-H25"
}
```

The server first sends a snapshot:

```json
{
  "type": "market_book_snapshot",
  "ticker": "SX-FEDDEC-26OCT-H25",
  "seq": 421,
  "book_seq": 421,
  "bids": [{ "price_ticks": 49, "total_qty": 120, "order_count": 8 }],
  "asks": [{ "price_ticks": 51, "total_qty": 96, "order_count": 6 }]
}
```

It then sends deltas:

```json
{
  "type": "market_book_delta",
  "event_id": "md-123",
  "ticker": "SX-FEDDEC-26OCT-H25",
  "global_seq": 8202,
  "contract_seq": 422,
  "book_seq": 422,
  "book_side": "BID",
  "side": 1,
  "price_ticks": 49,
  "qty_delta": -10,
  "new_total_qty": 110
}
```

`book_seq` is the continuity sequence for this market book. Clients must
apply a delta only when it follows the last applied `book_seq`; `new_total_qty`
is authoritative and `qty_delta` is a consistency check. A zero total removes
the level. On a gap, discard the local book and fetch a fresh REST snapshot
before applying newer deltas.

The current market channel emits book snapshots and book deltas. Use the REST
fills endpoint for public recent trades. The private channel below emits the
authenticated user's own execution events.

### Subscribe to private fills

Client message:

```json
{
  "op": "subscribe",
  "channel": "private",
  "ticker": "SX-FEDDEC-26OCT-H25"
}
```

The server only sends fills involving the authenticated user:

```json
{
  "type": "private_fill",
  "event_id": "fill-123",
  "ticker": "SX-FEDDEC-26OCT-H25",
  "global_seq": 8201,
  "contract_seq": 421,
  "order_id": "ord-123",
  "role": "taker",
  "price_ticks": 51,
  "count": 10,
  "aggressor_side": 1
}
```

Private events redact the counterparty's user and order identifiers.

WebSocket errors use compact messages such as:

```json
{ "type": "error", "code": "UNAUTHENTICATED" }
```

Known codes include `INVALID_MESSAGE`, `CHANNEL_REQUIRED`, `TICKER_REQUIRED`,
`UNAUTHENTICATED`, `SUBSCRIBE_FAILED`, `NATS_UNAVAILABLE`, and
`UNSUPPORTED_OPERATION`.

## Trading Client Rules

1. Fetch the contract before constructing an order and validate its state is
   `OPEN`.
2. Use integer `price_ticks` and `count`; do not send decimal prices.
3. Generate a new `client_order_id` and `Idempotency-Key` per logical action.
4. Treat the response order as authoritative and reconcile later using
   `GET /v1/orders` or `GET /v1/orders/{order_id}`.
5. For market orders, handle `MARKET_NOT_AVAILABLE` and partial IOC fills.
6. Do not assume every accepted order is filled.
7. Apply WebSocket deltas by sequence and recover from gaps with a REST
   snapshot.
8. Use ledger balances and position responses as integers in micro-USDC.
9. Never expose private fill/order fields received from authenticated feeds to
   another user.

## Current Scope and Deferred API Work

## RFQ API

RFQ endpoints use the same authenticated demo/JWT bearer token and require an
`Idempotency-Key` for every mutating request. Prices and quantities are integer
ticks/counts, matching the order API.

Create an RFQ:

```http
POST /v1/rfqs
Authorization: Bearer <token>
Idempotency-Key: rfq-create-123
Content-Type: application/json

{
  "client_rfq_id": "client-rfq-123",
  "ticker": "SX-FEDDEC-26OCT-H25",
  "side": "YES",
  "action": "BUY",
  "requested_count": 20,
  "expires_at": "2026-09-26T12:00:00Z"
}
```

Quote workflow:

```http
GET  /v1/rfqs/{rfq_id}
GET  /v1/rfqs/{rfq_id}/quotes?limit=100&cursor={cursor}
POST /v1/rfqs/{rfq_id}/quotes
POST /v1/rfqs/{rfq_id}/quotes/{quote_id}/accept
POST /v1/rfqs/{rfq_id}/quotes/{quote_id}/cancel
POST /v1/rfqs/{rfq_id}/cancel
```

Quote submission body:

```json
{
  "quote_id": "quote-123",
  "bid_price_ticks": 48,
  "offer_price_ticks": 52,
  "available_count": 20,
  "expires_at": "2026-09-26T11:59:00Z"
}
```

Accepting a quote atomically selects one quote and cancels the remaining
pending quotes. The initial implementation returns
`RFQ_STATUS_ACCEPTED_PENDING_EXECUTION`; it deliberately does not create a
fill outside me-core. RFQ execution is complete only after the future me-core
execution bridge submits the corresponding sequenced order flow and the RFQ
state is advanced to `EXECUTED`.

The following are intentionally not public REST endpoints yet:

- Order amendment (`AmendOrder` exists only in the internal protobuf service).
- User registration, password reset, and external identity-provider login.
- Production deposits and withdrawals.
- Oracle submission, resolution approval, and settlement administration.
- Arbitrary historical candles; charts currently derive from fills.
- A public event replay endpoint; replay is handled by snapshot plus sequence
  recovery and internal event retention.

These additions require their own authorization, idempotency, audit, and
rate-limit contracts before being exposed to third-party clients.
