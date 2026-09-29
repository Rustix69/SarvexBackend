import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import {
  BookOpen,
  Check,
  ChevronDown,
  ChevronRight,
  Copy,
  Command,
  Globe2,
  KeyRound,
  LockKeyhole,
  Search,
  Send,
  ShieldCheck,
  TerminalSquare,
  Wifi,
  X,
} from 'lucide-react'

const SAMPLE_TICKER = 'SX-FEDDEC-26OCT-H25'
const SAMPLE_EVENT = 'XEV-STD-ECO-01'

const field = (name, type, description, required = false) => ({ name, type, description, required })

const API_GROUPS = [
  {
    title: 'Getting started',
    tone: 'guide',
    items: [
      { id: 'guide-overview', title: 'API overview', kind: 'guide', description: 'HTTPS JSON endpoints for markets, accounts, orders and fills. Use the live console on any route to validate your integration.' },
      { id: 'guide-quickstart', title: 'Quickstart', kind: 'guide', description: 'Register or log in, create an API key, check that a market is open, then place an idempotent order and inspect its status.' },
      { id: 'guide-auth', title: 'Authentication', kind: 'guide', description: 'Private routes accept exactly one X-API-Key or Authorization: Bearer credential. The gateway derives the account from that credential.' },
      { id: 'guide-idempotency', title: 'Idempotency', kind: 'guide', description: 'Every mutating request uses a fresh Idempotency-Key. Reuse the same key only when retrying the same logical operation after a timeout.' },
      { id: 'guide-errors', title: 'Errors', kind: 'guide', description: 'Every error uses one stable envelope. Branch on error.code and log error.message.' },
      { id: 'guide-units', title: 'Prices & units', kind: 'guide', description: 'Prices, counts, sequence values and money are integer-based. Convert only at the display boundary.' },
    ],
  },
  {
    title: 'Health',
    tone: 'public',
    items: [
      { id: 'health-overview', title: 'Service health overview', method: 'GET', path: '/v1/health/overview', description: 'Aggregated health for the gateway and its dependent services.', response: '{\n  "summary": { "running": 10, "total": 10, "not_running": 0 },\n  "items": []\n}' },
      { id: 'healthz', title: 'Liveness check', method: 'GET', path: '/healthz', description: 'Lightweight unauthenticated liveness check.', response: '{ "status": "ok" }' },
      { id: 'readyz', title: 'Readiness check', method: 'GET', path: '/readyz', description: 'Readiness check for the gateway process.', response: '{ "status": "ready" }' },
    ],
  },
  {
    title: 'Market data',
    tone: 'public',
    items: [
      { id: 'markets', title: 'List markets', method: 'GET', path: '/v1/markets', description: 'List contracts with optional state, category, kind, series, event, underlying and cursor filters.', params: [field('state', 'string', 'OPEN, CLOSED, SETTLED or another contract state'), field('kind', 'string', 'BINARY, FUTURES or SCALAR'), field('category', 'string', 'Category filter such as Crypto or Economics'), field('series_ticker', 'string', 'Restrict to one series'), field('event_ticker', 'string', 'Restrict to one event'), field('limit', 'integer', '1 to 500; default 50'), field('cursor', 'string', 'Cursor returned by the previous page')], query: { state: 'OPEN', limit: '50' }, response: '{\n  "contracts": [],\n  "next_cursor": ""\n}' },
      { id: 'market', title: 'Get one market', method: 'GET', path: '/v1/markets/{ticker}', description: 'Fetch contract metadata. Validate state, price bounds and max order size before placing an order.', pathParams: ['ticker'], response: '{\n  "ticker": "SX-FEDDEC-26OCT-H25",\n  "question": "Will the FOMC raise the federal funds target range?",\n  "state": "OPEN",\n  "min_price_ticks": 1,\n  "max_price_ticks": 99,\n  "max_order_size": 100000\n}' },
      { id: 'orderbook', title: 'Order-book snapshot', method: 'GET', path: '/v1/markets/{ticker}/orderbook', description: 'Aggregated bids and asks. Use this snapshot to initialize or recover a local book after a WebSocket sequence gap.', pathParams: ['ticker'], params: [field('depth', 'integer', 'Levels per side, clamped to 1–100')], query: { depth: '20' }, response: '{\n  "ticker": "SX-FEDDEC-26OCT-H25",\n  "seq": 1842,\n  "bids": [{ "price_ticks": 49, "total_qty": 120, "order_count": 8 }],\n  "asks": [{ "price_ticks": 51, "total_qty": 96, "order_count": 6 }]\n}' },
      { id: 'market-fills', title: 'Market fills', method: 'GET', path: '/v1/markets/{ticker}/fills', description: 'Public anonymized trade history for a market. Use private fills for account reconciliation.', pathParams: ['ticker'], params: [field('from_global_seq', 'integer', 'Starting sequence'), field('to_global_seq', 'integer', 'Ending sequence'), field('limit', 'integer', 'Page size'), field('cursor', 'string', 'Pagination cursor')], query: { limit: '100' }, response: '{\n  "fills": [],\n  "next_cursor": ""\n}' },
      { id: 'open-interest', title: 'Open interest', method: 'GET', path: '/v1/markets/{ticker}/open-interest', description: 'Read total open long and short quantity for a contract.', pathParams: ['ticker'], response: '{\n  "ticker": "SX-FEDDEC-26OCT-H25",\n  "total_open_long": 120,\n  "total_open_short": 120,\n  "as_of_global_seq": 8201\n}' },
      { id: 'series', title: 'Discover series', method: 'GET', path: '/v1/series', description: 'List series that group related events and contracts.', params: [field('limit', 'integer', 'Page size'), field('cursor', 'string', 'Pagination cursor')], query: { limit: '100' }, response: '{\n  "series": [],\n  "next_cursor": ""\n}' },
      { id: 'events', title: 'Discover events', method: 'GET', path: '/v1/events', description: 'List real-world events that contracts settle against.', params: [field('series_ticker', 'string', 'Restrict to a series'), field('limit', 'integer', 'Page size'), field('cursor', 'string', 'Pagination cursor')], query: { limit: '100' }, response: '{\n  "events": [],\n  "next_cursor": ""\n}' },
      { id: 'event', title: 'Get one event', method: 'GET', path: '/v1/events/{event_ticker}', description: 'Fetch one event and its expected resolution metadata.', pathParams: ['event_ticker'], response: '{\n  "event_ticker": "XEV-STD-ECO-01",\n  "series_ticker": "XLSX-STANDARD",\n  "title": "Standard demo event",\n  "expected_resolution_at": "2026-10-28T23:59:00Z"\n}' },
      { id: 'resolution', title: 'Resolution and settlement', method: 'GET', path: '/v1/events/{event_ticker}/resolution', description: 'Read the public resolution state and outcome for an event.', pathParams: ['event_ticker'], response: '{\n  "event_ticker": "XEV-STD-ECO-01",\n  "status": "PENDING",\n  "outcome": null\n}' },
      { id: 'settlement', title: 'Get settlement result', method: 'GET', path: '/v1/markets/{ticker}/settlement', description: 'Read payout and settlement totals after a contract has settled.', pathParams: ['ticker'], response: '{\n  "ticker": "SX-FEDDEC-26OCT-H25",\n  "payout_per_contract_micro_usdc": 1000000,\n  "total_payout_micro_usdc": 0,\n  "positions_settled": 0\n}' },
    ],
  },
  {
    title: 'Authentication',
    tone: 'account',
    items: [
      { id: 'register', title: 'Create account', method: 'POST', path: '/v1/auth/register', description: 'Create a user account and receive a browser JWT.', body: { name: 'Alice Trader', user_id: 'alice_1', email: 'alice@example.com', password: 'a-long-password-12' }, bodyFields: [field('name', 'string', 'Display name', true), field('user_id', 'string', 'Unique account identifier', true), field('email', 'string', 'Unique email address', true), field('password', 'string', 'Password', true)], response: '{ "token": "<jwt>", "token_type": "Bearer", "user_id": "alice_1" }' },
      { id: 'login', title: 'Log in', method: 'POST', path: '/v1/auth/login', description: 'Exchange an email or user ID and password for a Bearer JWT.', body: { email: 'alice@example.com', password: 'a-long-password-12' }, bodyFields: [field('email', 'string', 'Email or send user_id instead', false), field('user_id', 'string', 'Alternative identifier', false), field('password', 'string', 'Password', true)], response: '{ "token": "<jwt>", "token_type": "Bearer", "user_id": "alice_1" }' },
      { id: 'profile', title: 'Get profile', method: 'GET', path: '/v1/account/profile', auth: true, description: 'Read the authenticated account profile. Password material is never returned.', response: '{ "user_id": "alice_1", "email": "alice@example.com", "status": "ACTIVE" }' },
      { id: 'api-keys', title: 'List API keys', method: 'GET', path: '/v1/account/api-keys', auth: true, description: 'List API-key metadata and prefixes. Plaintext secrets are only returned at creation time.', response: '{ "api_keys": [] }' },
      { id: 'create-api-key', title: 'Create API key', method: 'POST', path: '/v1/account/api-keys', auth: true, description: 'Create a user-scoped key for bots and external clients. Store the returned secret immediately.', body: { name: 'trading-bot', scopes: ['markets:read', 'account:read', 'orders:read', 'fills:read', 'trading:write'] }, bodyFields: [field('name', 'string', 'Key label', true), field('scopes', 'string[]', 'Least-privilege permissions', true), field('expires_at', 'RFC 3339', 'Optional expiry')], response: '{ "key": "svx_live_<shown-once>", "key_id": "key-123", "name": "trading-bot" }' },
      { id: 'revoke-api-key', title: 'Revoke API key', method: 'DELETE', path: '/v1/account/api-keys/{key_id}', auth: true, description: 'Revoke one key without affecting the account password or other keys.', pathParams: ['key_id'], response: '{ "revoked": true }' },
    ],
  },
  {
    title: 'Trading',
    tone: 'trading',
    items: [
      { id: 'orders', title: 'List orders', method: 'GET', path: '/v1/orders', auth: true, description: 'List the authenticated user’s orders. A successful submission may be OPEN, PARTIAL, FILLED, CANCELLED or REJECTED.', params: [field('ticker', 'string', 'Restrict to one contract'), field('status', 'string', 'Order status'), field('limit', 'integer', 'Page size'), field('cursor', 'string', 'Pagination cursor')], query: { limit: '100' }, response: '{ "orders": [], "next_cursor": "" }' },
      { id: 'order', title: 'Get one order', method: 'GET', path: '/v1/orders/{order_id}', auth: true, description: 'Fetch one order belonging to the authenticated account.', pathParams: ['order_id'], response: '{ "order": { "order_id": "ord-123", "status": "OPEN", "filled_count": 0, "remaining_count": 10 } }' },
      { id: 'submit-order', title: 'Submit order', method: 'POST', path: '/v1/orders', auth: true, idem: true, description: 'Place a limit or market order. Market orders use price_ticks 0 and are converted to protected IOC orders from the live opposite quote.', body: { client_order_id: 'client-order-000001', ticker: SAMPLE_TICKER, side: 'YES', action: 'BUY', order_type: 'LIMIT', price_ticks: 51, count: 10, tif: 'GTC' }, bodyFields: [field('client_order_id', 'string', 'Your reconciliation ID', true), field('ticker', 'string', 'Open contract ticker', true), field('side', 'string', 'Binary YES/NO; futures LONG/SHORT', true), field('action', 'string', 'BUY or SELL', true), field('order_type', 'string', 'LIMIT or MARKET'), field('price_ticks', 'integer', 'Positive for limit; 0 for MARKET', true), field('count', 'integer', 'Positive quantity', true), field('tif', 'string', 'GTC, IOC or FOK')], response: '{ "order": { "order_id": "ord-123", "status": "OPEN", "filled_count": 0, "remaining_count": 10 } }' },
      { id: 'cancel-order', title: 'Cancel order', method: 'POST', path: '/v1/orders/{order_id}/cancel', auth: true, idem: true, description: 'Cancel the remaining quantity of an open order.', pathParams: ['order_id'], response: '{ "order": { "order_id": "ord-123", "status": "CANCELLED" } }' },
    ],
  },
  {
    title: 'Account and portfolio',
    tone: 'account',
    items: [
      { id: 'balance', title: 'Get balance', method: 'GET', path: '/v1/account/balance', auth: true, description: 'Read available, held and total account balances.', response: '{ "available_micro_usdc": 10000000000, "held_micro_usdc": 0, "total_micro_usdc": 10000000000 }' },
      { id: 'risk', title: 'Get account risk', method: 'GET', path: '/v1/account/risk', auth: true, description: 'Aggregate exposure, collateral and limit usage for the authenticated account.', response: '{ "open_orders": 0, "open_positions": 0, "at_risk_micro_usdc": 0 }' },
      { id: 'fills', title: 'Private fill history', method: 'GET', path: '/v1/account/fills', auth: true, description: 'Reconcile fills involving only the authenticated account.', params: [field('ticker', 'string', 'Restrict to one contract'), field('order_id', 'string', 'Restrict to one order'), field('limit', 'integer', 'Page size'), field('cursor', 'string', 'Pagination cursor')], query: { limit: '100' }, response: '{ "fills": [], "next_cursor": "" }' },
      { id: 'history', title: 'Ledger history', method: 'GET', path: '/v1/account/history', auth: true, description: 'Read account ledger entries for reconciliation.', params: [field('limit', 'integer', 'Page size'), field('cursor', 'string', 'Pagination cursor')], query: { limit: '100' }, response: '{ "entries": [], "next_cursor": "" }' },
      { id: 'positions', title: 'List positions', method: 'GET', path: '/v1/positions', auth: true, description: 'List open and optionally closed positions.', params: [field('include_closed', 'boolean', 'Include closed positions'), field('limit', 'integer', 'Page size'), field('cursor', 'string', 'Pagination cursor')], query: { include_closed: 'false', limit: '100' }, response: '{ "positions": [], "next_cursor": "" }' },
      { id: 'position', title: 'Get one position', method: 'GET', path: '/v1/positions/{ticker}', auth: true, description: 'Fetch one position for the authenticated account and contract.', pathParams: ['ticker'], response: '{ "ticker": "SX-FEDDEC-26OCT-H25", "net_qty": 0, "avg_cost_micro_usdc": 0, "unrealized_pnl_micro_usdc": 0 }' },
      { id: 'deposit', title: 'Credit demo funds', method: 'POST', path: '/v1/demo/deposits/credit', auth: true, idem: true, description: 'Add test collateral on demo or devnet accounts. This route is not a real deposit rail.', body: { amount_usdc: 10000 }, bodyFields: [field('amount_usdc', 'number', 'Whole-dollar demo amount'), field('amount_micro_usdc', 'integer', 'Exact micro-USDC amount')], response: '{ "credited_micro_usdc": 10000000000 }' },
    ],
  },
  {
    title: 'Request for quote',
    tone: 'trading',
    items: [
      { id: 'create-rfq', title: 'Create RFQ', method: 'POST', path: '/v1/rfqs', auth: true, idem: true, description: 'Ask makers for a two-sided quote on a requested size.', body: { client_rfq_id: 'client-rfq-123', ticker: SAMPLE_TICKER, side: 'YES', action: 'BUY', requested_count: 20, expires_at: '2026-09-30T12:00:00Z' }, bodyFields: [field('client_rfq_id', 'string', 'Your reconciliation ID', true), field('ticker', 'string', 'Contract ticker', true), field('side', 'string', 'YES/NO or LONG/SHORT', true), field('action', 'string', 'BUY or SELL', true), field('requested_count', 'integer', 'Requested size', true), field('expires_at', 'RFC 3339', 'RFQ expiry', true)], response: '{ "rfq_id": "rfq-123", "status": "OPEN" }' },
      { id: 'get-rfq', title: 'Get RFQ', method: 'GET', path: '/v1/rfqs/{rfq_id}', auth: true, description: 'Read an RFQ owned by or visible to the authenticated account.', pathParams: ['rfq_id'], response: '{ "rfq_id": "rfq-123", "status": "OPEN", "quotes": [] }' },
      { id: 'rfq-quotes', title: 'List RFQ quotes', method: 'GET', path: '/v1/rfqs/{rfq_id}/quotes', auth: true, description: 'List quotes currently available for an RFQ.', pathParams: ['rfq_id'], query: { limit: '100' }, response: '{ "quotes": [], "next_cursor": "" }' },
      { id: 'submit-quote', title: 'Submit RFQ quote', method: 'POST', path: '/v1/rfqs/{rfq_id}/quotes', auth: true, idem: true, description: 'Submit a maker quote against an open RFQ.', pathParams: ['rfq_id'], body: { bid_price_ticks: 48, offer_price_ticks: 52, available_count: 20, expires_at: '2026-09-30T12:00:00Z' }, bodyFields: [field('bid_price_ticks', 'integer', 'Bid price', true), field('offer_price_ticks', 'integer', 'Offer price', true), field('available_count', 'integer', 'Available size', true), field('expires_at', 'RFC 3339', 'Quote expiry', true)], response: '{ "quote_id": "quote-123", "status": "PENDING" }' },
      { id: 'accept-quote', title: 'Accept RFQ quote', method: 'POST', path: '/v1/rfqs/{rfq_id}/quotes/{quote_id}/accept', auth: true, idem: true, description: 'Accept a quote and create the corresponding trade workflow.', pathParams: ['rfq_id', 'quote_id'], response: '{ "rfq_id": "rfq-123", "status": "ACCEPTED" }' },
      { id: 'cancel-quote', title: 'Cancel RFQ quote', method: 'POST', path: '/v1/rfqs/{rfq_id}/quotes/{quote_id}/cancel', auth: true, idem: true, description: 'Cancel a quote that is still pending.', pathParams: ['rfq_id', 'quote_id'], response: '{ "quote_id": "quote-123", "status": "CANCELLED" }' },
      { id: 'cancel-rfq', title: 'Cancel RFQ', method: 'POST', path: '/v1/rfqs/{rfq_id}/cancel', auth: true, idem: true, description: 'Cancel an open RFQ.', pathParams: ['rfq_id'], response: '{ "rfq_id": "rfq-123", "status": "CANCELLED" }' },
    ],
  },
  {
    title: 'WebSocket',
    tone: 'stream',
    items: [
      { id: 'ws-connect', title: 'Connect and authenticate', kind: 'guide', method: 'WS', path: 'wss://api.sarvaex.com/ws', description: 'Connect to the WebSocket gateway. Public market subscriptions need no credential. Private fills use Authorization: Bearer <token> on the upgrade request.' },
      { id: 'ws-market', title: 'Market channel', kind: 'guide', method: 'WS', path: 'channel: market', description: 'Subscribe with { op: subscribe, channel: market, ticker }. The server sends a snapshot, then book deltas and anonymized market trades. Apply deltas only when book_seq is exactly one greater than your local sequence.' },
      { id: 'ws-private', title: 'Private fills channel', kind: 'guide', method: 'WS', path: 'channel: private', description: 'Subscribe with the authenticated account to receive only your own fills. After a disconnect, reconcile from GET /v1/account/fills using the last global sequence.' },
    ],
  },
]

const API_ITEMS = API_GROUPS.flatMap((group) => group.items.map((item) => ({ ...item, group: group.title, tone: group.tone })))

function randomId(prefix) {
  const suffix = typeof crypto !== 'undefined' && crypto.randomUUID ? crypto.randomUUID() : `${Date.now()}-${Math.random().toString(16).slice(2)}`
  return `${prefix}-${suffix}`
}

function monotonicNow() {
  return typeof performance !== 'undefined' ? performance.now() : Date.now()
}

function initialDraft(item) {
  const path = {}
  ;(item.pathParams || []).forEach((key) => {
    path[key] = key === 'ticker' ? SAMPLE_TICKER : key === 'event_ticker' ? SAMPLE_EVENT : key === 'order_id' ? 'ord-123' : key === 'rfq_id' ? 'rfq-123' : key === 'quote_id' ? 'quote-123' : key === 'key_id' ? 'key-123' : ''
  })
  return { path, query: { ...(item.query || {}) }, body: item.body ? JSON.stringify(item.body, null, 2) : '', idem: item.idem ? randomId('svx') : '', result: null }
}

function resolvePath(path, params) {
  return path.replace(/\{(\w+)\}/g, (_, key) => encodeURIComponent(params[key] || `{${key}}`))
}

function displayBase(baseUrl) {
  if (baseUrl.startsWith('http')) return baseUrl
  return `${window.location.origin}${baseUrl}`
}

function parseDraftBody(draft) {
  if (!draft.body) return undefined
  try { return JSON.parse(draft.body) } catch { return null }
}

function pythonLiteral(value) {
  return JSON.stringify(value, null, 2)
    .replace(/\btrue\b/g, 'True')
    .replace(/\bfalse\b/g, 'False')
    .replace(/\bnull\b/g, 'None')
}

function authHeader(credentialMode, credential) {
  if (credentialMode === 'apikey') return ['X-API-Key', credential || 'svx_live_<your-key>']
  return ['Authorization', `Bearer ${credential || '<jwt>'}`]
}

function generatedPython({ method, url, auth, credentialMode, credential, idem, body }) {
  const headers = []
  if (auth) {
    const [name, value] = authHeader(credentialMode, credential)
    headers.push(`        "${name}": "${value}",`)
  }
  if (idem) headers.push(`        "Idempotency-Key": "${idem}",`)
  if (body !== undefined) headers.push('        "Content-Type": "application/json",')
  const lines = ['import requests', '', `resp = requests.${method.toLowerCase()}(`, `    "${url}",`]
  if (headers.length) lines.push('    headers={', ...headers, '    },')
  if (body !== undefined) lines.push(`    json=${pythonLiteral(body).split('\n').join('\n    ')},`)
  lines.push('    timeout=10,', ')', 'resp.raise_for_status()', 'print(resp.status_code, resp.text)')
  return lines.join('\n')
}

function generatedJavaScript({ method, url, auth, credentialMode, credential, idem, body }) {
  const headers = []
  if (auth) {
    const [name, value] = authHeader(credentialMode, credential)
    headers.push(`    "${name}": "${value}",`)
  }
  if (idem) headers.push(`    "Idempotency-Key": "${idem}",`)
  if (body !== undefined) headers.push('    "Content-Type": "application/json",')
  const lines = [`const res = await fetch("${url}", {`]
  if (method !== 'GET') lines.push(`  method: "${method}",`)
  if (headers.length) lines.push('  headers: {', ...headers, '  },')
  if (body !== undefined) lines.push(`  body: JSON.stringify(${JSON.stringify(body, null, 2).split('\n').join('\n  ')}),`)
  lines.push('});', 'console.log(res.status, await res.text());')
  return lines.join('\n')
}

export default function TradingApiPage({ baseUrl, token }) {
  const [selectedId, setSelectedId] = useState('guide-overview')
  const [query, setQuery] = useState('')
  const [searchOpen, setSearchOpen] = useState(false)
  const [searchIndex, setSearchIndex] = useState(0)
  const [credentialMode, setCredentialMode] = useState(token ? 'bearer' : 'apikey')
  const [credential, setCredential] = useState(token || '')
  const [drafts, setDrafts] = useState({})
  const [copied, setCopied] = useState('')
  const [codeTab, setCodeTab] = useState('response')
  const [wsTicker, setWsTicker] = useState(SAMPLE_TICKER)
  const [wsState, setWsState] = useState({ status: 'disconnected', events: [], error: '' })
  const wsRef = useRef(null)
  const searchInputRef = useRef(null)
  const referenceRef = useRef(null)

  const selected = API_ITEMS.find((item) => item.id === selectedId) || API_ITEMS[0]
  const draft = drafts[selected.id] || initialDraft(selected)
  const filteredGroups = useMemo(() => {
    const normalized = query.trim().toLowerCase()
    if (!normalized) return API_GROUPS
    return API_GROUPS.map((group) => ({ ...group, items: group.items.filter((item) => `${group.title} ${item.title} ${item.path || ''} ${item.description}`.toLowerCase().includes(normalized)) })).filter((group) => group.items.length)
  }, [query])
  const searchResults = useMemo(() => API_ITEMS.filter((item) => {
    const normalized = query.trim().toLowerCase()
    if (!normalized) return true
    return `${item.group} ${item.title} ${item.path || ''} ${item.description || ''}`.toLowerCase().includes(normalized)
  }), [query])

  const updateDraft = (changes) => setDrafts((current) => {
    const currentDraft = current[selected.id] || initialDraft(selected)
    return { ...current, [selected.id]: { ...currentDraft, ...changes } }
  })
  const updatePath = (key, value) => updateDraft({ path: { ...draft.path, [key]: value } })
  const updateQuery = (key, value) => updateDraft({ query: { ...draft.query, [key]: value } })

  const isRestEndpoint = Boolean(selected.path && selected.method !== 'WS' && selected.kind !== 'guide')
  const requestUrl = isRestEndpoint ? `${baseUrl}${resolvePath(selected.path, draft.path)}${Object.entries(draft.query || {}).filter(([, value]) => value !== '' && value != null).length ? `?${new URLSearchParams(Object.entries(draft.query).filter(([, value]) => value !== '' && value != null)).toString()}` : ''}` : ''
  const curl = selected.path ? `curl -X ${selected.method} '${displayBase(requestUrl)}'${selected.auth ? ` \\\n  -H '${credentialMode === 'apikey' ? 'X-API-Key' : 'Authorization'}: ${credentialMode === 'apikey' ? credential || 'svx_live_<your-key>' : `Bearer ${credential || '<jwt>'}`}'` : ''}${selected.idem ? ` \\\n  -H 'Idempotency-Key: ${draft.idem}'` : ''}${selected.body ? ` \\\n  -H 'Content-Type: application/json' \\\n  -d '${draft.body.replace(/'/g, "'\\''")}'` : ''}` : ''

  const selectItem = useCallback((item) => {
    setSelectedId(item.id)
    setDrafts((current) => current[item.id] ? current : ({ ...current, [item.id]: initialDraft(item) }))
    setSearchOpen(false)
    setQuery('')
    setSearchIndex(0)
  }, [])

  const selectedIndex = API_ITEMS.findIndex((item) => item.id === selected.id)
  const selectRelative = (offset) => {
    const next = API_ITEMS[selectedIndex + offset]
    if (next) selectItem(next)
  }

  const wsUrl = baseUrl.startsWith('http')
    ? `${baseUrl.replace(/^http/, 'ws')}/ws`
    : `${window.location.protocol === 'https:' ? 'wss' : 'ws'}://${window.location.hostname}:18082/ws`

  const disconnectWebSocket = () => {
    wsRef.current?.close()
    wsRef.current = null
    setWsState((current) => ({ ...current, status: 'disconnected' }))
  }

  const connectWebSocket = () => {
    disconnectWebSocket()
    setWsState({ status: 'connecting', events: [], error: '' })
    const socket = new WebSocket(wsUrl)
    wsRef.current = socket
    socket.onopen = () => {
      setWsState((current) => ({ ...current, status: 'connected' }))
      socket.send(JSON.stringify({ op: 'subscribe', channel: 'market', ticker: wsTicker.trim() }))
    }
    socket.onmessage = (event) => {
      let message = event.data
      try { message = JSON.parse(event.data) } catch { /* keep plain text */ }
      setWsState((current) => ({ ...current, events: [...current.events, message].slice(-40) }))
    }
    socket.onerror = () => setWsState((current) => ({ ...current, status: 'error', error: 'WebSocket connection failed.' }))
    socket.onclose = () => {
      wsRef.current = null
      setWsState((current) => ({ ...current, status: 'disconnected' }))
    }
  }

  useEffect(() => () => wsRef.current?.close(), [])

  useEffect(() => {
    const onKeyDown = (event) => {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'k') {
        event.preventDefault()
        setSearchOpen(true)
        window.setTimeout(() => searchInputRef.current?.focus(), 0)
        return
      }
      if (!searchOpen) return
      if (event.key === 'Escape') {
        event.preventDefault()
        setSearchOpen(false)
      } else if (event.key === 'ArrowDown') {
        event.preventDefault()
        setSearchIndex((current) => searchResults.length ? (current + 1) % searchResults.length : 0)
      } else if (event.key === 'ArrowUp') {
        event.preventDefault()
        setSearchIndex((current) => searchResults.length ? (current - 1 + searchResults.length) % searchResults.length : 0)
      } else if (event.key === 'Enter' && searchResults[searchIndex]) {
        event.preventDefault()
        selectItem(searchResults[searchIndex])
      }
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [searchOpen, searchIndex, searchResults, selectItem])

  useEffect(() => {
    if (referenceRef.current) referenceRef.current.scrollTop = 0
  }, [selectedId])

  const runRequest = async () => {
    if (!selected.path) return
    if (selected.auth && !credential.trim()) {
      updateDraft({ result: { status: 401, duration: 0, body: { error: { code: 'MISSING_CREDENTIAL', message: 'Enter an API key or Bearer token before running this private request.' } } } })
      return
    }
    let body
    if (selected.body) {
      try {
        body = draft.body.trim() ? JSON.parse(draft.body) : {}
      } catch (error) {
        updateDraft({ result: { status: 400, duration: 0, body: { error: { code: 'INVALID_JSON', message: error.message } } } })
        return
      }
    }
    const headers = {}
    if (body !== undefined) headers['Content-Type'] = 'application/json'
    if (selected.auth) headers[credentialMode === 'apikey' ? 'X-API-Key' : 'Authorization'] = credentialMode === 'apikey' ? credential.trim() : `Bearer ${credential.trim()}`
    if (selected.idem) headers['Idempotency-Key'] = draft.idem || randomId('svx')
    const started = monotonicNow()
    updateDraft({ result: { pending: true } })
    try {
      const response = await fetch(requestUrl, { method: selected.method, headers, body: body === undefined ? undefined : JSON.stringify(body) })
      const text = await response.text()
      let parsed = text
      try { parsed = text ? JSON.parse(text) : null } catch { /* plain text response */ }
      updateDraft({ result: { status: response.status, duration: Math.round(monotonicNow() - started), body: parsed } })
      if (selected.idem && response.ok) updateDraft({ idem: randomId('svx') })
    } catch (error) {
      updateDraft({ result: { status: 0, duration: Math.round(monotonicNow() - started), body: { error: { code: 'NETWORK_ERROR', message: error.message } } } })
    }
  }

  const copyText = async (key, value) => {
    try { await navigator.clipboard.writeText(value); setCopied(key); window.setTimeout(() => setCopied(''), 1400) } catch { setCopied('') }
  }

  const codeBody = parseDraftBody(draft)
  const codeSamples = {
    curl,
    python: isRestEndpoint ? generatedPython({ method: selected.method, url: displayBase(requestUrl), auth: selected.auth, credentialMode, credential, idem: selected.idem ? draft.idem : '', body: codeBody }) : '',
    javascript: isRestEndpoint ? generatedJavaScript({ method: selected.method, url: displayBase(requestUrl), auth: selected.auth, credentialMode, credential, idem: selected.idem ? draft.idem : '', body: codeBody }) : '',
  }

  return (
    <main className="api-docs-page">
      <header className="api-docs-header">
        <div className="api-docs-title"><span className="api-docs-icon"><TerminalSquare size={18} /></span><div><h1>Sarvaex <span>API TERMINAL</span></h1></div></div>
        <button className="api-global-search" type="button" onClick={() => { setSearchIndex(0); setSearchOpen(true); window.setTimeout(() => searchInputRef.current?.focus(), 0) }}><Search size={14} /><span>Jump to an endpoint or guide</span><kbd><Command size={10} /> K</kbd></button>
        <div className="api-docs-base"><Globe2 size={14} /> <span>{displayBase(baseUrl)}</span><span className="api-live-dot">Live</span></div>
      </header>
      <div className="api-docs-mobile-tabs"><button type="button" className="active"><BookOpen size={14} /> Reference</button><button type="button"><TerminalSquare size={14} /> Console</button></div>
      <div className="api-docs-layout">
        <aside className="api-docs-sidebar">
          <button className="api-sidebar-search" type="button" onClick={() => { setSearchIndex(0); setSearchOpen(true); window.setTimeout(() => searchInputRef.current?.focus(), 0) }}><Search size={14} /><span>Jump to an endpoint or guide</span><kbd>⌘K</kbd></button>
          <div className="api-index-meta"><span>API reference</span><strong>{API_ITEMS.filter((item) => item.path).length} routes</strong></div>
          <nav className="api-endpoint-tree" aria-label="API endpoints">
            {filteredGroups.map((group) => <div className="api-tree-group" key={group.title}>
              <div className="api-tree-heading"><ChevronDown size={13} /> <span>{group.title}</span><em>{group.items.length}</em></div>
              {group.items.map((item) => <button className={`api-tree-item ${selected.id === item.id ? 'active' : ''}`} type="button" key={item.id} onClick={() => selectItem(item)}><span className={`api-method ${item.kind === 'guide' ? 'doc' : item.method.toLowerCase()}`}>{item.kind === 'guide' ? item.method || 'DOC' : item.method}</span><span>{item.title}</span></button>)}
            </div>)}
          </nav>
        </aside>

        <section className="api-docs-reference" ref={referenceRef}>
          <div className="api-breadcrumb"><span>{selected.group}</span><ChevronRight size={13} /><span>{selected.kind === 'guide' ? 'Guide' : 'Endpoint reference'}</span></div>
          <div className="api-reference-heading"><div><h2>{selected.title}</h2>{selected.path ? <div className="api-route"><span className={`api-method ${selected.method.toLowerCase()}`}>{selected.method}</span><code>{selected.path}</code></div> : null}</div><span className={`api-auth-tag ${selected.auth ? 'private' : 'public'}`}>{selected.auth ? <><LockKeyhole size={12} /> Authenticated</> : <><Globe2 size={12} /> Public</>}</span></div>
          {selected.kind === 'guide' ? <GuideContent item={selected} /> : <EndpointContent item={selected} />}
          <div className="api-reference-pager"><button type="button" disabled={selectedIndex <= 0} onClick={() => selectRelative(-1)}><small>← Previous</small><span>{API_ITEMS[selectedIndex - 1]?.title || ''}</span></button><button type="button" disabled={selectedIndex >= API_ITEMS.length - 1} onClick={() => selectRelative(1)}><small>Next →</small><span>{API_ITEMS[selectedIndex + 1]?.title || ''}</span></button></div>
        </section>

        <aside className="api-docs-console">
          <div className="api-console-heading"><div><span className="api-docs-kicker">Developer tools</span><h2>Console</h2></div><span className="api-console-status"><span /> {isRestEndpoint ? 'REST' : 'Guide'}</span></div>
          {isRestEndpoint ? <>
            <div className="api-console-url"><span>{selected.method}</span><code>{requestUrl}</code><button className="api-console-run-compact" type="button" onClick={runRequest}><Send size={12} /> Run <kbd>Ctrl ↵</kbd></button></div>
            {selected.auth ? <div className="api-console-block"><div className="api-console-label"><span><ShieldCheck size={13} /> Credential</span><small>one header</small></div><div className="api-credential-switch"><button type="button" className={credentialMode === 'apikey' ? 'active' : ''} onClick={() => setCredentialMode('apikey')}><KeyRound size={12} /> API key</button><button type="button" className={credentialMode === 'bearer' ? 'active' : ''} onClick={() => setCredentialMode('bearer')}>Bearer</button></div><input className="api-console-input" type={credentialMode === 'apikey' ? 'password' : 'text'} value={credential} onChange={(event) => setCredential(event.target.value)} placeholder={credentialMode === 'apikey' ? 'svx_live_...' : '<jwt>'} autoComplete="off" /></div> : null}
            {(selected.pathParams || []).length ? <div className="api-console-block"><div className="api-console-label"><span>Path parameters</span><small>required</small></div>{selected.pathParams.map((key) => <label className="api-console-field" key={key}><span>{key}</span><input value={draft.path[key] || ''} onChange={(event) => updatePath(key, event.target.value)} /></label>)}</div> : null}
            {(selected.params || []).length ? <div className="api-console-block"><div className="api-console-label"><span>Query parameters</span><small>empty fields omitted</small></div>{selected.params.map((param) => <label className="api-console-field" key={param.name}><span>{param.name}</span><input value={draft.query[param.name] || ''} onChange={(event) => updateQuery(param.name, event.target.value)} placeholder="optional" /></label>)}</div> : null}
            {selected.idem ? <div className="api-console-block"><div className="api-console-label"><span>Idempotency-Key</span><button type="button" className="api-inline-button" onClick={() => updateDraft({ idem: randomId('svx') })}>New key</button></div><input className="api-console-input mono" value={draft.idem} onChange={(event) => updateDraft({ idem: event.target.value })} /></div> : null}
            {selected.body ? <div className="api-console-block"><div className="api-console-label"><span>JSON body</span><small>application/json</small></div><textarea className="api-body-editor" value={draft.body} onChange={(event) => updateDraft({ body: event.target.value })} spellCheck="false" /></div> : null}
            <RequestOutput tab={codeTab} onTabChange={setCodeTab} result={draft.result} samples={codeSamples} onCopy={copyText} copied={copied} />
          </> : selected.method === 'WS' ? <WebSocketConsole wsUrl={wsUrl} ticker={wsTicker} onTickerChange={setWsTicker} state={wsState} onConnect={connectWebSocket} onDisconnect={disconnectWebSocket} /> : <div className="api-console-guide"><Wifi size={20} /><strong>Reference only</strong><p>This section documents the streaming contract. Use the REST console for live requests; connect to the WebSocket URL from your application.</p><code>wss://api.sarvaex.com/ws</code></div>}
        </aside>
      </div>
      {searchOpen ? <div className="api-search-overlay" role="presentation" onMouseDown={(event) => { if (event.target === event.currentTarget) setSearchOpen(false) }}><div className="api-search-dialog" role="dialog" aria-modal="true" aria-label="Search API endpoints"><div className="api-search-dialog-input"><Search size={17} /><input ref={searchInputRef} value={query} onChange={(event) => { setQuery(event.target.value); setSearchIndex(0) }} placeholder="Search endpoints, paths, guides" aria-label="Search endpoints, paths, guides" /><button type="button" onClick={() => { setQuery(''); setSearchOpen(false) }} aria-label="Close search"><X size={16} /></button></div><div className="api-search-results">{searchResults.length ? searchResults.map((item, index) => <button type="button" className={`api-search-result ${index === searchIndex ? 'active' : ''}`} key={item.id} onMouseEnter={() => setSearchIndex(index)} onClick={() => selectItem(item)}><span className={`api-method ${item.kind === 'guide' ? 'doc' : item.method.toLowerCase()}`}>{item.kind === 'guide' ? 'DOC' : item.method}</span><strong>{item.title}</strong><small>{item.kind === 'guide' ? item.group : item.path}</small></button>) : <div className="api-search-empty">No matching endpoints or guides.</div>}</div><div className="api-search-footer"><span><kbd>↑</kbd><kbd>↓</kbd> navigate</span><span><kbd>↵</kbd> open</span><span><kbd>esc</kbd> close</span></div></div></div> : null}
    </main>
  )
}

function GuideContent({ item }) {
  if (item.id === 'guide-units') return <UnitsGuide />
  if (item.id === 'guide-errors') return <ErrorGuide />
  const guideBlocks = {
    'guide-overview': [['Base URL', 'Production: https://api.sarvaex.com\nLocal Docker: http://localhost:18080'], ['Request model', 'JSON over HTTPS. Prices, counts and monetary values use integer units defined by the contract.'], ['Integration path', 'Fetch contract metadata, initialize the order book, submit idempotent orders, then reconcile private fills and positions.']],
    'guide-quickstart': [['1. Authenticate', 'POST /v1/auth/register or POST /v1/auth/login to receive a JWT.'], ['2. Create a bot key', 'POST /v1/account/api-keys with only the scopes your integration needs. The plaintext key is shown once.'], ['3. Validate and trade', 'GET /v1/markets/{ticker}, confirm state is OPEN, then POST /v1/orders with X-API-Key and a fresh Idempotency-Key. Read order.status after submission.']],
    'guide-auth': [['Public routes', 'Health, market data, series, events and settlement reads do not require authentication.'], ['API keys', 'Send X-API-Key: svx_live_<secret>. Keys are user-scoped and the plaintext secret is shown only once.'], ['Browser sessions', 'Send Authorization: Bearer <jwt>. Never send a user_id to act as another account.']],
    'guide-idempotency': [['Mutating requests', 'Orders, cancellations, RFQs, quotes and demo credits require Idempotency-Key.'], ['Retry rule', 'Reuse the same key for a retry of the same logical operation. Generate a fresh key for a new operation.'], ['Expected result', 'A successful HTTP response is not the same as a fill. Read order status, filled_count and remaining_count.']],
    'guide-errors': [['Error envelope', '{\n  "error": {\n    "code": "INVALID_ARGUMENT",\n    "message": "count must be positive"\n  }\n}'], ['400', 'Invalid request, enum, price, count or contract state.'], ['401 / 403', 'Missing or invalid credentials, or authenticated but not permitted.'], ['404 / 409', 'Resource not found, idempotency conflict or state conflict.'], ['502 / 503 / 504', 'Internal dependency failure, service unavailable or upstream deadline exceeded.']],
    'ws-connect': [['Connection', 'wss://api.sarvaex.com/ws'], ['Authentication', 'Public market subscriptions need no credential. Private fills use Authorization: Bearer <token> on the upgrade request.'], ['First message', '{ "type": "connected", "service": "gw-ws" }']],
    'ws-market': [['Subscribe', '{ "op": "subscribe", "channel": "market", "ticker": "SX-FEDDEC-26OCT-H25" }'], ['Events', 'market_book_snapshot, market_book_delta and market_trade. Use global_seq for cross-event ordering.'], ['Recovery', 'On a book_seq gap, discard the local book, fetch the REST snapshot, then apply newer deltas.']],
    'ws-private': [['Subscribe', '{ "op": "subscribe", "channel": "private", "ticker": "SX-FEDDEC-26OCT-H25" }'], ['Events', 'private_fill events contain your order and fill details while redacting counterparty identifiers.'], ['Reconciliation', 'After disconnect, reconcile with GET /v1/account/fills from the last known global sequence.']],
  }
  return <div className="api-guide-content"><p>{item.description}</p>{(guideBlocks[item.id] || []).map(([title, text]) => <div className="api-guide-block" key={title}><h3>{title}</h3><pre>{text}</pre></div>)}</div>
}

function ErrorGuide() {
  const rows = [['400', 'Invalid request, enum, price, count or contract state'], ['401', 'Missing or invalid Bearer token or API key'], ['403', 'Authenticated but not permitted'], ['404', 'Contract, order or position not found'], ['409', 'Idempotency conflict or state conflict'], ['502', 'Internal service or matching-engine failure'], ['503', 'Gateway, database or matching engine unavailable'], ['504', 'Upstream deadline exceeded, where applicable']]
  return <div className="api-guide-content"><p>Every error uses one envelope. Branch on <code>code</code> and log <code>message</code>.</p><div className="api-error-example"><h3>Error envelope</h3><pre>{'{\n  "error": {\n    "code": "INVALID_ARGUMENT",\n    "message": "count must be positive"\n  }\n}'}</pre></div><div className="api-error-table"><div><strong>Status</strong><strong>Meaning</strong></div>{rows.map(([status, meaning]) => <div key={status}><code>{status}</code><span>{meaning}</span></div>)}</div></div>
}

function UnitsGuide() {
  const [price, setPrice] = useState(51)
  const costMicro = price * 10 * 10000
  return <div className="api-guide-content"><p>On binary contracts <code>price_ticks</code> usually means cents. YES at <code>49</code> is $0.49, the complementary NO is $0.51, and a winning contract pays $1.00. Futures and scalar contracts convert with <code>divider</code>, <code>tick_value_micro</code> and multiplier fields instead.</p><p>Money is micro-USDC. Divide by <code>1_000_000</code> only when displaying.</p><div className="api-unit-slider"><label htmlFor="api-price-ticks">Drag price_ticks</label><input id="api-price-ticks" type="range" min="1" max="99" value={price} onChange={(event) => setPrice(Number(event.target.value))} /><div className="api-unit-bar"><span style={{ width: `${price}%` }} /></div><div className="api-unit-values"><strong>YES {price}¢</strong><strong>NO {100 - price}¢</strong></div></div><div className="api-unit-table"><span>For 10 contracts</span><div><strong>Cost</strong><b>${(costMicro / 1_000_000).toFixed(2)} · {costMicro.toLocaleString()} micro-USDC</b></div><div><strong>Pays if right</strong><b>$10.00 · 10,000,000 micro-USDC</b></div></div><div className="api-guide-block"><h3>Python conversion</h3><pre>{'MICRO = 1_000_000\n\ndef ticks_to_usd(price_ticks: int) -> str:\n    return f"${price_ticks / 100:.2f}"\n\ndef micro_to_usdc(micro: int) -> str:\n    return f"{micro / MICRO:,.2f} USDC"'}</pre></div></div>
}

function RequestOutput({ tab, onTabChange, result, samples, onCopy, copied }) {
  const isCode = tab !== 'response'
  const responseText = result ? JSON.stringify(result.body, null, 2) : '// Run the request to see the live response.'
  const outputText = isCode ? samples[tab] : responseText
  return <div className="api-output"><div className="api-output-tabs" role="tablist" aria-label="Generated request output">{[['response', 'Response'], ['curl', 'cURL'], ['python', 'Python'], ['javascript', 'JavaScript']].map(([value, label]) => <button key={value} type="button" className={tab === value ? 'active' : ''} onClick={() => onTabChange(value)}>{label}</button>)}<button className="api-output-copy" type="button" onClick={() => onCopy(tab, outputText)}>{copied === tab ? <Check size={12} /> : <Copy size={12} />} {copied === tab ? 'Copied' : 'Copy'}</button></div>{tab === 'response' && result ? <div className="api-response-status"><strong className={result.status >= 200 && result.status < 300 ? 'ok' : 'error'}>{result.status || 'ERR'}</strong><span>{result.status >= 200 && result.status < 300 ? 'Request completed' : 'Request failed'} · {result.duration} ms</span></div> : isCode ? <div className="api-generated-label"><span>{tab === 'curl' ? 'shell' : tab}</span><span>Generated from the fields above</span></div> : null}<pre className={isCode ? 'api-code-output' : 'api-response-output'}>{outputText}</pre></div>
}

function EndpointContent({ item }) {
  return <div className="api-endpoint-content"><p>{item.description}</p>{item.params?.length ? <FieldTable title="Query parameters" fields={item.params} /> : null}{item.bodyFields?.length ? <FieldTable title="JSON body" fields={item.bodyFields} /> : null}<div className="api-response-example"><div className="api-section-title"><span>Example response</span><span className="api-muted">{item.auth ? 'Authenticated response' : 'Public response'}</span></div><pre>{item.response || '{ "ok": true }'}</pre></div>{item.idem ? <div className="api-warning"><ShieldCheck size={15} /><span><strong>Idempotent write.</strong> Send a unique Idempotency-Key and inspect the returned order or RFQ state.</span></div> : null}</div>
}

function FieldTable({ title, fields }) {
  return <div className="api-field-table"><div className="api-section-title"><span>{title}</span><span className="api-muted">{fields.length} fields</span></div>{fields.map((entry) => <div className="api-field-row" key={entry.name}><div><code>{entry.name}</code>{entry.required ? <b>required</b> : null}</div><span>{entry.type}</span><p>{entry.description}</p></div>)}</div>
}

function WebSocketConsole({ wsUrl, ticker, onTickerChange, state, onConnect, onDisconnect }) {
  const connected = state.status === 'connected' || state.status === 'connecting'
  return <div className="api-ws-console"><div className="api-ws-url"><Wifi size={14} /><code>{wsUrl}</code></div><div className="api-console-block"><div className="api-console-label"><span>Public market channel</span><small>no credential required</small></div><label className="api-console-field"><span>ticker</span><input value={ticker} onChange={(event) => onTickerChange(event.target.value)} disabled={connected} /></label></div><div className="api-ws-actions">{connected ? <button className="api-ws-disconnect" type="button" onClick={onDisconnect}>Disconnect</button> : <button className="api-run-button" type="button" onClick={onConnect}><Wifi size={14} /> Connect and subscribe</button>}<span className={`api-ws-state ${state.status}`}>{state.status}</span></div>{state.error ? <p className="api-ws-error">{state.error}</p> : null}<div className="api-ws-log"><div className="api-console-label"><span>Event log</span><small>{state.events.length} events</small></div>{state.events.length ? <pre>{state.events.map((event, index) => `${JSON.stringify(event, null, 2)}${index < state.events.length - 1 ? '\n\n' : ''}`).join('')}</pre> : <div className="api-response-empty">Connect to receive connected, snapshot, delta and trade events.</div>}</div><div className="api-ws-note"><ShieldCheck size={14} /><span>Browser tester covers the public market channel. Private fills require an application WebSocket client that sends the Bearer header during upgrade.</span></div></div>
}
