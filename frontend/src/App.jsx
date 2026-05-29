/* eslint-disable react-hooks/set-state-in-effect */
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import {
  Activity,
  ArrowLeft,
  BarChart3,
  Bookmark,
  ChevronDown,
  CircleDollarSign,
  Clock3,
  Gift,
  Link2,
  Loader2,
  LogOut,
  RefreshCw,
  Share2,
} from 'lucide-react'
import './App.css'

const API_BASE = import.meta.env.VITE_API_BASE_URL || '/api'
const DEMO_MAX_ORDER_CENTS = 10000
const LIVE_TRADE_REFRESH_MS = 650
const LIVE_PAGE_REFRESH_MS = 1200
const SCALAR_KIND = 2
const DEMO_USERS = [
  { id: 'u_retail_1', label: 'Demo Retail', badge: 'Retail' },
  { id: 'u_mm_1', label: 'Market Maker', badge: 'MM' },
  { id: 'u_inst_1', label: 'Institutional', badge: 'Inst' },
  { id: 'u_admin', label: 'Demo Admin', badge: 'Admin' },
]

function viewFromPath(pathname) {
  if (pathname === '/health') return 'health'
  if (pathname === '/futures') return 'futures'
  if (pathname === '/portfolio') return 'portfolio'
  return 'markets'
}

function pathForView(view) {
  if (view === 'health') return '/health'
  if (view === 'futures') return '/futures'
  if (view === 'portfolio') return '/portfolio'
  return '/'
}

function pushViewPath(view) {
  const nextPath = pathForView(view)
  if (window.location.pathname !== nextPath) {
    window.history.pushState({}, '', nextPath)
  }
}

function App() {
  const [token, setToken] = useState(() => localStorage.getItem('sarvex_token') || '')
  const [userId, setUserId] = useState(() => localStorage.getItem('sarvex_user_id') || 'u_retail_1')
  const [markets, setMarkets] = useState([])
  const [futures, setFutures] = useState([])
  const [selectedTicker, setSelectedTicker] = useState('')
  const [orderbook, setOrderbook] = useState(null)
  const [fills, setFills] = useState([])
  const [balance, setBalance] = useState(null)
  const [positions, setPositions] = useState([])
  const [orders, setOrders] = useState([])
  const [bookMarks, setBookMarks] = useState({})
  const [loading, setLoading] = useState(true)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const [activeView, setActiveView] = useState(() => viewFromPath(window.location.pathname))
  const fillCursorRef = useRef({})

  const selectedMarket = useMemo(
    () => [...markets, ...futures].find((market) => market.ticker === selectedTicker),
    [futures, markets, selectedTicker],
  )
  const selectedIsFuture = isFutureMarket(selectedMarket)
  const marketPrices = useMemo(() => {
    const prices = {}
    ;[...markets, ...futures].forEach((market) => {
      prices[market.ticker] = impliedPrice(market, fills)
    })
    return { ...prices, ...bookMarks }
  }, [bookMarks, fills, futures, markets])
  const marketByTicker = useMemo(() => {
    const byTicker = {}
    ;[...markets, ...futures].forEach((market) => {
      byTicker[market.ticker] = market
    })
    return byTicker
  }, [futures, markets])
  const selectedPosition = useMemo(
    () => positions.find((position) => position.ticker === selectedTicker),
    [positions, selectedTicker],
  )

  const authed = Boolean(token)

  const api = useCallback(
    async (path, options = {}) => {
      const headers = { 'Content-Type': 'application/json', ...(options.headers || {}) }
      if (token) headers.Authorization = `Bearer ${token}`
      const response = await fetch(`${API_BASE}${path}`, { ...options, headers })
      const text = await response.text()
      const body = text ? JSON.parse(text) : null
      if (!response.ok) {
        const message = body?.error?.message || body?.message || `Request failed: ${response.status}`
        throw new Error(message)
      }
      return body
    },
    [token],
  )

  const login = useCallback(
    async (nextUserId = userId) => {
      setBusy(true)
      setError('')
      try {
        const body = await fetch(`${API_BASE}/v1/auth/login`, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ user_id: nextUserId }),
        }).then(async (response) => {
          const data = await response.json()
          if (!response.ok) throw new Error(data?.error?.message || 'Login failed')
          return data
        })
        localStorage.setItem('sarvex_token', body.token)
        localStorage.setItem('sarvex_user_id', nextUserId)
        setToken(body.token)
        setUserId(nextUserId)
      } catch (err) {
        setError(err.message)
      } finally {
        setBusy(false)
      }
    },
    [userId],
  )

  const fetchMarketFills = useCallback(async (ticker) => {
    const incoming = []
    let cursor = fillCursorRef.current[ticker] || ''
    const maxPages = cursor ? 2 : 20

    for (let page = 0; page < maxPages; page += 1) {
      const query = new URLSearchParams({ limit: '500' })
      if (cursor) query.set('cursor', cursor)
      const body = await api(`/v1/markets/${ticker}/fills?${query.toString()}`)
      const pageFills = body?.fills || []
      incoming.push(...pageFills)
      const nextCursor = body?.next_cursor || body?.nextCursor || ''
      if (!nextCursor) break
      cursor = nextCursor
    }

    const maxSeq = incoming.reduce((max, fill) => Math.max(max, fillSeq(fill)), Number(fillCursorRef.current[ticker] || 0))
    if (maxSeq > 0) fillCursorRef.current[ticker] = String(maxSeq)
    return incoming
  }, [api])

  const refreshPublic = useCallback(async () => {
    setError('')
    const marketBody = await api('/v1/markets?state=OPEN&limit=50')
    const contracts = marketBody?.contracts || []
    const nextMarkets = contracts.filter((market) => !isFutureMarket(market))
    const nextFutures = contracts.filter((market) => isFutureMarket(market)).slice(0, 9)
    setMarkets(nextMarkets)
    setFutures(nextFutures)
    const visibleContracts = [...nextMarkets, ...nextFutures]
    const ticker = selectedTicker || nextMarkets[0]?.ticker || nextFutures[0]?.ticker
    const fillsByMarket = await Promise.all(visibleContracts.map((market) => fetchMarketFills(market.ticker).catch(() => [])))
    const incomingFills = fillsByMarket.flat()
    setFills((current) => mergeRecentFills(current, incomingFills, visibleContracts.map((market) => market.ticker)))
    if (!ticker) return
    setOrderbook(await api(`/v1/markets/${ticker}/orderbook?depth=12`))
  }, [api, fetchMarketFills, selectedTicker])

  const refreshBookMarks = useCallback(async (nextPositions = [], nextOrders = []) => {
    const tickers = new Set()
    nextPositions.forEach((position) => {
      if (position?.ticker) tickers.add(position.ticker)
    })
    nextOrders.forEach((order) => {
      if (order?.ticker && activeOrderStatus(order.status)) tickers.add(order.ticker)
    })
    if (!tickers.size) {
      setBookMarks({})
      return
    }

    const entries = await Promise.all([...tickers].map(async (ticker) => {
      const book = await api(`/v1/markets/${ticker}/orderbook?depth=1`)
      const bid = Number(book?.bids?.[0]?.price_ticks || book?.bids?.[0]?.priceTicks || 0)
      const ask = Number(book?.asks?.[0]?.price_ticks || book?.asks?.[0]?.priceTicks || 0)
      const mark = midpoint(bid, ask) || ask || bid || 0
      return [ticker, mark]
    }))
    setBookMarks(Object.fromEntries(entries.filter(([, mark]) => mark > 0)))
  }, [api])

  const refreshPrivate = useCallback(async () => {
    if (!token) return
    const [balanceBody, positionsBody, ordersBody] = await Promise.all([
      api('/v1/account/balance'),
      api('/v1/positions?include_closed=true'),
      api('/v1/orders?limit=20'),
    ])
    const nextPositions = positionsBody?.positions || []
    const nextOrders = ordersBody?.orders || []
    setBalance(balanceBody)
    setPositions(nextPositions)
    setOrders(nextOrders)
    refreshBookMarks(nextPositions, nextOrders).catch(() => {})
  }, [api, refreshBookMarks, token])

  const refreshAll = useCallback(async () => {
    setLoading(true)
    try {
      await refreshPublic()
      await refreshPrivate()
    } catch (err) {
      setError(err.message)
    } finally {
      setLoading(false)
    }
  }, [refreshPrivate, refreshPublic])

  useEffect(() => {
    refreshAll()
  }, [refreshAll])

  useEffect(() => {
    const onPopState = () => {
      setSelectedTicker('')
      setActiveView(viewFromPath(window.location.pathname))
    }
    window.addEventListener('popstate', onPopState)
    return () => window.removeEventListener('popstate', onPopState)
  }, [])

  useEffect(() => {
    const refresh = () => {
      refreshPublic().catch((err) => setError(err.message))
      refreshPrivate().catch(() => {})
    }
    const interval = setInterval(() => {
      refresh()
    }, activeView === 'trade' ? LIVE_TRADE_REFRESH_MS : LIVE_PAGE_REFRESH_MS)
    return () => clearInterval(interval)
  }, [activeView, refreshPrivate, refreshPublic])

  const handleMarketSelect = (ticker) => {
    setSelectedTicker(ticker)
    setActiveView('trade')
    if (window.location.pathname === '/health') window.history.pushState({}, '', '/')
    window.scrollTo({ top: 0, behavior: 'smooth' })
  }

  const handleMarketsNav = () => {
    setSelectedTicker('')
    setActiveView('markets')
    pushViewPath('markets')
    window.scrollTo({ top: 0, behavior: 'smooth' })
  }

  const handleViewNav = (view) => {
    setSelectedTicker('')
    setActiveView(view)
    pushViewPath(view)
    window.scrollTo({ top: 0, behavior: 'smooth' })
  }

  const handleDeposit = async () => {
    setBusy(true)
    setError('')
    try {
      await api('/v1/demo/deposits/credit', {
        method: 'POST',
        body: JSON.stringify({ amount_usdc: 10000, note: 'frontend quick fund' }),
      })
      await refreshPrivate()
    } catch (err) {
      setError(err.message)
    } finally {
      setBusy(false)
    }
  }

  const handleExitPosition = async (position) => {
    const ticker = position?.ticker
    const qty = positionQty(position)
    if (!ticker || !qty) return

    setBusy(true)
    setError('')
    try {
      const book = await api(`/v1/markets/${ticker}/orderbook?depth=1`)
      const bestBid = Number(book?.bids?.[0]?.price_ticks || book?.bids?.[0]?.priceTicks || 0)
      const bestAsk = Number(book?.asks?.[0]?.price_ticks || book?.asks?.[0]?.priceTicks || 0)
      const market = marketByTicker[ticker]
      const scalar = isFutureMarket(market)
      const action = qty > 0 ? 'SELL' : 'BUY'
      const priceTicks = action === 'SELL' ? bestBid : bestAsk
      if (!priceTicks) throw new Error(`No exit liquidity available for ${ticker}`)

      let remaining = Math.abs(qty)
      const maxChunk = scalar ? remaining : maxExitOrderCount(action, priceTicks)
      let chunkIndex = 0

      while (remaining > 0) {
        const count = Math.min(remaining, maxChunk)
        const id = `exit-${Date.now()}-${chunkIndex}-${Math.random().toString(16).slice(2)}`
        const result = await api('/v1/orders', {
          method: 'POST',
          headers: { 'Idempotency-Key': id },
          body: JSON.stringify({
            client_order_id: id,
            ticker,
            side: scalar ? 'LONG' : 'YES',
            action,
            price_ticks: scalar ? clampFutureTicks(market, priceTicks) : Math.max(1, Math.min(99, Math.round(priceTicks))),
            count,
            tif: 'GTC',
            reduce_only: true,
          }),
        })
        const rejected = orderRejectMessage(result)
        if (rejected) throw new Error(rejected)
        remaining -= count
        chunkIndex += 1
      }

      await refreshAll()
    } catch (err) {
      setError(err.message)
    } finally {
      setBusy(false)
    }
  }

  const selectedUser = DEMO_USERS.find((user) => user.id === userId) || DEMO_USERS[0]
  const selectedFills = selectedMarket ? fills.filter((fill) => fill.ticker === selectedMarket.ticker) : []

  return (
    <div className="sarvex-shell">
      <TopNav
        selectedUser={selectedUser}
        userId={userId}
        setUserId={setUserId}
        token={token}
        busy={busy}
        onLogin={login}
        onMarkets={handleMarketsNav}
        onNavigateView={handleViewNav}
        activeView={activeView}
      />

      {error && <div className="notice error">{error}</div>}

      {activeView === 'trade' && selectedMarket && selectedIsFuture ? (
        <FutureDetail
          market={selectedMarket}
          orderbook={orderbook}
          fills={selectedFills}
          position={selectedPosition}
          authed={authed}
          busy={busy}
          onBack={() => handleViewNav('futures')}
          onTrade={async (payload) => {
            setBusy(true)
            setError('')
            try {
              const result = await api('/v1/orders', {
                method: 'POST',
                headers: { 'Idempotency-Key': payload.client_order_id },
                body: JSON.stringify(payload),
              })
              const rejected = orderRejectMessage(result)
              if (rejected) throw new Error(rejected)
              await refreshAll()
            } catch (err) {
              setError(err.message)
            } finally {
              setBusy(false)
            }
          }}
        />
      ) : activeView === 'trade' && selectedMarket ? (
        <MarketDetail
          market={selectedMarket}
          orderbook={orderbook}
          fills={selectedFills}
          position={selectedPosition}
          authed={authed}
          busy={busy}
          onBack={handleMarketsNav}
          onTrade={async (payload) => {
            setBusy(true)
            setError('')
            try {
              const result = await api('/v1/orders', {
                method: 'POST',
                headers: { 'Idempotency-Key': payload.client_order_id },
                body: JSON.stringify(payload),
              })
              const rejected = orderRejectMessage(result)
              if (rejected) throw new Error(rejected)
              await refreshAll()
            } catch (err) {
              setError(err.message)
            } finally {
              setBusy(false)
            }
          }}
        />
      ) : activeView === 'portfolio' ? (
        <PortfolioPage
          balance={balance}
          authed={authed}
          busy={busy}
          positions={positions}
          orders={orders}
          marketPrices={marketPrices}
          marketByTicker={marketByTicker}
          selectedUser={selectedUser}
          onDeposit={handleDeposit}
          onRefresh={refreshPrivate}
          onExitPosition={handleExitPosition}
        />
      ) : activeView === 'health' ? (
        <HealthPage api={api} />
      ) : activeView === 'futures' ? (
        <FuturesDashboard
          loading={loading}
          futures={futures}
          fills={fills}
          onSelect={handleMarketSelect}
          onRefresh={refreshAll}
        />
      ) : (
        <MarketDashboard
          loading={loading}
          markets={markets}
          fills={fills}
          onSelect={handleMarketSelect}
          onRefresh={refreshAll}
        />
      )}
    </div>
  )
}

function TopNav({ selectedUser, userId, setUserId, token, busy, onLogin, onMarkets, onNavigateView, activeView }) {
  return (
    <header className="topbar">
      <button className="brand" type="button" onClick={onMarkets}>
        <img className="brand-mark" src="/logo.png" alt="" aria-hidden="true" />
        <span>SarvaEX</span>
      </button>
      <div />
      <div className="user-cluster">
        <button className={activeView === 'markets' ? 'nav-link active' : 'nav-link'} type="button" onClick={onMarkets}>Markets</button>
        <button className={activeView === 'futures' ? 'nav-link active' : 'nav-link'} type="button" onClick={() => onNavigateView('futures')}>Futures</button>
        <button className={activeView === 'portfolio' ? 'nav-link active' : 'nav-link'} type="button" onClick={() => onNavigateView('portfolio')}>Portfolio</button>
        <button className={activeView === 'health' ? 'nav-link active' : 'nav-link'} type="button" onClick={() => onNavigateView('health')}>Health</button>
        <select value={userId} onChange={(event) => setUserId(event.target.value)}>
          {DEMO_USERS.map((user) => (
            <option value={user.id} key={user.id}>{user.label}</option>
          ))}
        </select>
        <button className="demo-login-btn" type="button" onClick={() => onLogin(userId)} disabled={busy}>
          {busy ? <Loader2 className="spin" size={15} /> : null}{token ? selectedUser.label : 'Log in demo'}
        </button>
      </div>
    </header>
  )
}

function HealthPage({ api }) {
  const [health, setHealth] = useState(null)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState('')

  const loadHealth = useCallback(async () => {
    try {
      const body = await api('/v1/health/overview')
      setHealth(body)
      setError('')
    } catch (err) {
      setError(err.message)
    } finally {
      setLoading(false)
    }
  }, [api])

  useEffect(() => {
    loadHealth()
    const interval = setInterval(loadHealth, 2500)
    return () => clearInterval(interval)
  }, [loadHealth])

  const items = health?.items || []
  const summary = health?.summary || {}

  return (
    <main className="health-page">
      <section className="health-hero">
        <div>
          <p className="portfolio-kicker">SarvaEX system status</p>
          <h1>Health</h1>
        </div>
        <button className="secondary-btn refresh-btn" type="button" onClick={loadHealth}><RefreshCw size={16} /> Refresh</button>
      </section>

      <section className="health-summary">
        <div>
          <span>Running</span>
          <strong>{summary.running ?? 0}</strong>
        </div>
        <div>
          <span>Not running</span>
          <strong className={summary.not_running ? 'pnl-negative' : 'pnl-flat'}>{summary.not_running ?? 0}</strong>
        </div>
        <div>
          <span>Total checks</span>
          <strong>{summary.total ?? items.length}</strong>
        </div>
      </section>

      <section className="health-panel">
        <div className="panel-head">
          <h2>Backend and simulators</h2>
          <span>{health?.generated_at ? `Updated ${formatTime(health.generated_at)}` : 'Waiting for status'}</span>
        </div>
        {error ? <div className="notice error health-error">{error}</div> : null}
        {loading ? (
          <div className="loading-panel"><Loader2 className="spin" /> Loading health checks...</div>
        ) : (
          <div className="health-table">
            <div className="health-row health-header"><span>Service</span><span>Type</span><span>Status</span><span>Latency</span><span>Error / detail</span></div>
            {items.map((item) => (
              <div className="health-row" key={`${item.kind}-${item.name}`}>
                <span>{item.name}</span>
                <span>{item.kind}</span>
                <span className={item.status === 'running' ? 'health-status running' : 'health-status down'}>
                  <i /> {item.status === 'running' ? 'Running' : 'Not running'}
                </span>
                <span>{Number.isFinite(Number(item.latency_ms)) ? `${item.latency_ms} ms` : '--'}</span>
                <span title={item.target || ''}>{item.message || '--'}</span>
              </div>
            ))}
          </div>
        )}
      </section>
    </main>
  )
}

function MarketDashboard({ loading, markets, fills, onSelect, onRefresh }) {
  const rows = markets.length ? markets : []
  return (
    <main className="dashboard-page">
      <section className="dashboard-hero">
        <div>
          <h1>Trade real-world outcomes with USDC-settled order books.</h1>
        </div>
        <button className="secondary-btn refresh-btn" type="button" onClick={onRefresh}><RefreshCw size={16} /> Refresh markets</button>
      </section>

      {loading ? (
        <div className="loading-panel"><Loader2 className="spin" /> Loading Sarvex markets...</div>
      ) : (
        <section className="market-grid">
          {rows.map((market, index) => (
            <MarketCard
              key={market.ticker}
              market={market}
              fills={fills}
              index={index}
              onClick={() => onSelect(market.ticker)}
            />
          ))}
        </section>
      )}
    </main>
  )
}

function MarketCard({ market, fills, index, onClick }) {
  const price = impliedPrice(market, fills)
  const options = [
    { label: 'Yes', price },
    { label: 'No', price: 100 - price },
  ]
  return (
    <button className="market-card" type="button" onClick={onClick} style={{ animationDelay: `${index * 35}ms` }}>
      <div className="card-topline">
        <div className="market-avatar">{avatarText(market)}</div>
        <h3>{market.question || market.underlying || market.ticker}</h3>
      </div>
      <div className="outcome-list">
        {options.map((option) => (
          <div className="outcome-row" key={option.label}>
            <span>{option.label}</span>
            <strong key={`${option.label}-${option.price}`}>{option.price}%</strong>
          </div>
        ))}
      </div>
    </button>
  )
}

function FuturesDashboard({ loading, futures, fills, onSelect, onRefresh }) {
  const rows = futures.length ? futures : []
  return (
    <main className="dashboard-page">
      <section className="dashboard-hero">
        <div>
          <h1>Trade the number the world will print.</h1>
        </div>
        <button className="secondary-btn refresh-btn" type="button" onClick={onRefresh}><RefreshCw size={16} /> Refresh futures</button>
      </section>

      {loading ? (
        <div className="loading-panel"><Loader2 className="spin" /> Loading Sarvex futures...</div>
      ) : rows.length ? (
        <section className="market-grid">
          {rows.map((market, index) => (
            <FutureCard
              key={market.ticker}
              market={market}
              fills={fills}
              index={index}
              onClick={() => onSelect(market.ticker)}
            />
          ))}
        </section>
      ) : (
        <div className="loading-panel">No numeric futures are open yet.</div>
      )}
    </main>
  )
}

function FutureCard({ market, fills, index, onClick }) {
  const price = impliedPrice(market, fills)
  return (
    <button className="market-card future-card" type="button" onClick={onClick} style={{ animationDelay: `${index * 35}ms` }}>
      <div className="card-topline">
        <div className="market-avatar">{avatarText(market)}</div>
        <h3>{market.question || market.underlying || market.ticker}</h3>
      </div>
      <div className="outcome-list future-list">
        <div className="outcome-row future-current-price">
          <span>Current price</span>
          <strong key={price}>{formatFuturePrice(market, price)}</strong>
        </div>
        <div className="outcome-row">
          <span>Range</span>
          <strong>{formatFutureRange(market)}</strong>
        </div>
      </div>
    </button>
  )
}

function MarketDetail({ market, orderbook, fills, position, authed, busy, onBack, onTrade }) {
  const bestBid = Number(orderbook?.bids?.[0]?.price_ticks || orderbook?.bids?.[0]?.priceTicks || 0)
  const bestAsk = Number(orderbook?.asks?.[0]?.price_ticks || orderbook?.asks?.[0]?.priceTicks || 0)
  const last = Number(fills?.[fills.length - 1]?.price_ticks || fills?.[fills.length - 1]?.priceTicks || bestAsk || bestBid || 50)
  const chartPoints = useMemo(() => buildChartPoints(fills, last), [fills, last])

  return (
    <main className="detail-page">
      <section className="market-main">
        <button className="back-btn" type="button" onClick={onBack}><ArrowLeft size={16} /> All markets</button>
        <div className="detail-heading">
          <div className="market-avatar xl">{avatarText(market)}</div>
          <div>
            <p className="crumb">{market.series_ticker || market.seriesTicker || 'Sarvex'} · {market.kind === 2 ? 'Scalar Future' : 'Binary Contract'}</p>
            <h1>{market.question || market.underlying || market.ticker}</h1>
          </div>
          <div className="heading-actions"><Share2 size={18} /><Link2 size={18} /><Bookmark size={18} /></div>
        </div>

        <div className="metric-row">
          <span>Last traded: <strong>{last}¢</strong></span>
          <span><Clock3 size={15} /> {formatDate(market.close_at || market.closeAt || market.expected_resolution_at)}</span>
          <span><Activity size={15} /> {fills.length} recent fills</span>
        </div>

        <section className="chart-card">
          <div className="chart-header">
            <div><span>Implied chance</span><strong>{last}%</strong></div>
            <span className="powered">Powered by Sarvex ME</span>
          </div>
          <svg className="price-chart" viewBox="0 0 720 260" role="img" aria-label="Market price chart">
            <defs>
              <linearGradient id="priceFill" x1="0" x2="0" y1="0" y2="1">
                <stop offset="0%" stopColor="#4f46e5" stopOpacity="0.22" />
                <stop offset="100%" stopColor="#4f46e5" stopOpacity="0" />
              </linearGradient>
            </defs>
            <path d={`${chartPoints.area} L 720 248 L 0 248 Z`} fill="url(#priceFill)" />
            <path d={chartPoints.line} fill="none" stroke="#4f46e5" strokeWidth="3" strokeLinecap="round" />
            {[0, 1, 2, 3].map((line) => <line key={line} x1="0" x2="720" y1={28 + line * 56} y2={28 + line * 56} stroke="#ececf2" />)}
          </svg>
        </section>

        <section className="contracts-table">
          <ContractRow title="Yes" price={bestAsk || last} side="yes" onClick={() => document.querySelector('#trade-ticket')?.scrollIntoView({ behavior: 'smooth' })} />
          <ContractRow title="No" price={100 - (bestBid || last)} side="no" onClick={() => document.querySelector('#trade-ticket')?.scrollIntoView({ behavior: 'smooth' })} />
        </section>

        <section className="lower-grid">
          <OrderBook book={orderbook} market={market} />
          <RecentTrades fills={fills} market={market} />
        </section>
      </section>

      <aside className="trade-side">
        <TradeTicket
          market={market}
          bestBid={bestBid}
          bestAsk={bestAsk}
          authed={authed}
          busy={busy}
          onTrade={onTrade}
        />
        <PositionSnapshot position={position} mark={last} market={market} authed={authed} />
      </aside>
    </main>
  )
}

function FutureDetail({ market, orderbook, fills, position, authed, busy, onBack, onTrade }) {
  const bestBid = Number(orderbook?.bids?.[0]?.price_ticks || orderbook?.bids?.[0]?.priceTicks || 0)
  const bestAsk = Number(orderbook?.asks?.[0]?.price_ticks || orderbook?.asks?.[0]?.priceTicks || 0)
  const last = Number(fills?.[fills.length - 1]?.price_ticks || fills?.[fills.length - 1]?.priceTicks || midpoint(bestBid, bestAsk) || futureFallbackTicks(market))
  const chartPoints = useMemo(() => buildChartPoints(fills, last, market), [fills, last, market])
  const multiplier = Number(market.multiplier_micro_usdc ?? market.multiplierMicroUsdc ?? 0)

  return (
    <main className="detail-page">
      <section className="market-main">
        <button className="back-btn" type="button" onClick={onBack}><ArrowLeft size={16} /> All futures</button>
        <div className="detail-heading">
          <div className="market-avatar xl">{avatarText(market)}</div>
          <div>
            <p className="crumb">{market.series_ticker || market.seriesTicker || 'Sarvex'} · Numeric Future</p>
            <h1>{market.question || market.underlying || market.ticker}</h1>
          </div>
          <div className="heading-actions"><Share2 size={18} /><Link2 size={18} /><Bookmark size={18} /></div>
        </div>

        <div className="metric-row">
          <span>Current price: <strong>{formatFuturePrice(market, last)}</strong></span>
          <span><Clock3 size={15} /> {formatDate(market.close_at || market.closeAt || market.expected_resolution_at)}</span>
          <span><Activity size={15} /> {fills.length} recent fills</span>
          <span>Tick: <strong>{formatFutureTick(market)}</strong></span>
        </div>

        <section className="chart-card">
          <div className="chart-header">
            <div><span>Market price</span><strong>{formatFuturePrice(market, last)}</strong></div>
            <span className="powered">Linear USDC-settled demo future</span>
          </div>
          <svg className="price-chart" viewBox="0 0 720 260" role="img" aria-label="Futures price chart">
            <defs>
              <linearGradient id="futurePriceFill" x1="0" x2="0" y1="0" y2="1">
                <stop offset="0%" stopColor="#08784e" stopOpacity="0.2" />
                <stop offset="100%" stopColor="#08784e" stopOpacity="0" />
              </linearGradient>
            </defs>
            <path d={`${chartPoints.area} L 720 248 L 0 248 Z`} fill="url(#futurePriceFill)" />
            <path d={chartPoints.line} fill="none" stroke="#08784e" strokeWidth="3" strokeLinecap="round" />
            {[0, 1, 2, 3].map((line) => <line key={line} x1="0" x2="720" y1={28 + line * 56} y2={28 + line * 56} stroke="#ececf2" />)}
          </svg>
        </section>

        <section className="contracts-table future-contracts">
          <button className="contract-row" type="button" onClick={() => document.querySelector('#trade-ticket')?.scrollIntoView({ behavior: 'smooth' })}>
            <div><strong>Long higher</strong><span>Payoff rises when final value is above entry.</span></div>
            <b>{formatFuturePrice(market, bestAsk || last)}</b>
            <em className="yes">Buy / Long</em>
          </button>
          <button className="contract-row" type="button" onClick={() => document.querySelector('#trade-ticket')?.scrollIntoView({ behavior: 'smooth' })}>
            <div><strong>Short lower</strong><span>Payoff rises when final value is below entry.</span></div>
            <b>{formatFuturePrice(market, bestBid || last)}</b>
            <em className="no">Sell / Short</em>
          </button>
        </section>

        <section className="lower-grid">
          <OrderBook book={orderbook} market={market} />
          <RecentTrades fills={fills} market={market} />
        </section>
      </section>

      <aside className="trade-side">
        <FutureTradeTicket
          market={market}
          bestBid={bestBid}
          bestAsk={bestAsk}
          mark={last}
          authed={authed}
          busy={busy}
          onTrade={onTrade}
        />
        <PositionSnapshot position={position} mark={last} market={market} authed={authed} />
        <section className="position-card">
          <div className="panel-head compact"><h2>Contract spec</h2><span>Demo v1</span></div>
          <div className="position-stat-grid">
            <div><span>Range</span><strong>{formatFutureRange(market)}</strong></div>
            <div><span>Multiplier</span><strong>{formatFutureMultiplierSpec(market, multiplier)}</strong></div>
          </div>
        </section>
      </aside>
    </main>
  )
}

function ContractRow({ title, price, side, onClick }) {
  return (
    <button className="contract-row" type="button" onClick={onClick}>
      <div><strong>{title}</strong><span>{side === 'yes' ? '$10,420 Vol.' : '$7,180 Vol.'}</span></div>
      <b key={price}>{price}%</b>
      <em className={side === 'yes' ? 'yes' : 'no'}>Buy {title} {Math.max(1, Math.min(99, price))}¢</em>
    </button>
  )
}

function TradeTicket({ market, bestBid, bestAsk, authed, busy, onTrade }) {
  const [tab, setTab] = useState('buy')
  const [outcome, setOutcome] = useState('YES')
  const [amount, setAmount] = useState('10')
  const suggestedYes = bestAsk || bestBid || 50
  const price = outcome === 'YES' ? suggestedYes : 100 - suggestedYes
  const priceTicks = Math.max(1, Math.min(99, Math.round(price || 50)))
  const spend = Math.max(0, Number(amount || 0))
  const requestedShares = spend > 0 ? Math.floor(spend / (priceTicks / 100)) : 0
  const maxShares = Math.max(1, Math.floor(DEMO_MAX_ORDER_CENTS / priceTicks))
  const shares = requestedShares > 0 ? Math.max(1, Math.min(requestedShares, maxShares)) : 0
  const capped = requestedShares > maxShares
  const estimatedSpend = shares * priceTicks / 100

  const updateAmount = (value) => {
    const nextAmount = Number(value || 0)
    setAmount(String(Math.min(100, Math.max(0, nextAmount))))
  }

  const submit = () => {
    if (!shares) return
    const yesPrice = outcome === 'YES' ? price : 100 - price
    const action = outcome === 'YES'
      ? (tab === 'buy' ? 'BUY' : 'SELL')
      : (tab === 'buy' ? 'SELL' : 'BUY')
    const id = `fe-${Date.now()}-${Math.random().toString(16).slice(2)}`
    onTrade({
      client_order_id: id,
      ticker: market.ticker,
      side: 'YES',
      action,
      price_ticks: Math.max(1, Math.min(99, Math.round(yesPrice))),
      count: shares,
      tif: 'GTC',
    })
  }

  return (
    <section className="ticket" id="trade-ticket">
      <div className="ticket-market"><div className="market-avatar small">{avatarText(market)}</div><span>{market.ticker}</span></div>
      <div className="ticket-tabs">
        <button className={tab === 'buy' ? 'active' : ''} type="button" onClick={() => setTab('buy')}>Buy</button>
        <button className={tab === 'sell' ? 'active' : ''} type="button" onClick={() => setTab('sell')}>Sell</button>
        <span>Limit <ChevronDown size={14} /></span>
      </div>
      <div className="outcome-toggle">
        <button className={outcome === 'YES' ? 'yes active' : 'yes'} type="button" onClick={() => setOutcome('YES')}>Yes {suggestedYes}¢</button>
        <button className={outcome === 'NO' ? 'no active' : 'no'} type="button" onClick={() => setOutcome('NO')}>No {100 - suggestedYes}¢</button>
      </div>
      <label className="amount-input">
        <span>Amount</span>
        <input value={amount} onChange={(event) => setAmount(event.target.value.replace(/[^\d.]/g, ''))} inputMode="decimal" />
      </label>
      <div className="quick-amounts">
        {[1, 5, 10, 25].map((value) => <button type="button" key={value} onClick={() => updateAmount(Number(amount || 0) + value)}>+${value}</button>)}
      </div>
      <div className="ticket-summary"><span>Est. shares</span><strong>{shares}</strong></div>
      <div className="ticket-summary muted"><span>Est. spend</span><strong>${estimatedSpend.toFixed(2)}</strong></div>
      {capped ? <div className="ticket-note">Demo cap applied at $100 per order.</div> : null}
      <button className="trade-btn" type="button" disabled={!authed || busy || !shares} onClick={submit}>
        {busy ? <Loader2 className="spin" size={17} /> : <CircleDollarSign size={17} />} {authed ? 'Trade' : 'Login to trade'}
      </button>
      <p>By trading, you agree to Sarvex demo terms.</p>
    </section>
  )
}

function FutureTradeTicket({ market, bestBid, bestAsk, mark, authed, busy, onTrade }) {
  const [tab, setTab] = useState('buy')
  const [priceInput, setPriceInput] = useState(() => formatFutureInput(market, bestAsk || bestBid || mark))
  const [size, setSize] = useState('5')
  const parsedPrice = parseFutureInput(market, priceInput)
  const priceTicks = parsedPrice > 0 ? clampFutureTicks(market, parsedPrice) : 0
  const contracts = Math.max(0, Math.floor(Number(size || 0)))
  const hold = computeFutureHoldMicro(market, tab, priceTicks, contracts)
  const payoffNote = tab === 'buy'
    ? 'Final payoff: (final value - entry) x contracts x multiplier.'
    : 'Final payoff: (entry - final value) x contracts x multiplier.'
  const switchSide = (nextTab) => {
    setTab(nextTab)
    const nextPrice = nextTab === 'buy' ? (bestAsk || mark || bestBid) : (bestBid || mark || bestAsk)
    if (nextPrice) setPriceInput(formatFutureInput(market, nextPrice))
  }

  const submit = () => {
    if (!contracts || !priceTicks) return
    const id = `fut-${Date.now()}-${Math.random().toString(16).slice(2)}`
    onTrade({
      client_order_id: id,
      ticker: market.ticker,
      side: 'LONG',
      action: tab === 'buy' ? 'BUY' : 'SELL',
      price_ticks: priceTicks,
      count: contracts,
      tif: 'GTC',
    })
  }

  return (
    <section className="ticket futures-ticket" id="trade-ticket">
      <div className="ticket-market"><div className="market-avatar small">{avatarText(market)}</div><span>{market.ticker}</span></div>
      <div className="ticket-tabs">
        <button className={tab === 'buy' ? 'active long-tab' : 'long-tab'} type="button" onClick={() => switchSide('buy')}>Buy / Long</button>
        <button className={tab === 'sell' ? 'active short-tab' : 'short-tab'} type="button" onClick={() => switchSide('sell')}>Sell / Short</button>
        <span>Limit <ChevronDown size={14} /></span>
      </div>
      <label className="amount-input future-price-input">
        <span>Entry price</span>
        <input value={priceInput} onChange={(event) => setPriceInput(event.target.value.replace(/[^\d.]/g, ''))} inputMode="decimal" />
      </label>
      <label className="amount-input future-size-input">
        <span>Contracts</span>
        <input value={size} onChange={(event) => setSize(event.target.value.replace(/[^\d]/g, ''))} inputMode="numeric" />
      </label>
      <div className="quick-amounts">
        {[1, 5, 10, 25].map((value) => <button type="button" key={value} onClick={() => setSize(String(Math.max(0, Number(size || 0)) + value))}>+{value}</button>)}
      </div>
      <div className="ticket-summary"><span>Entry</span><strong>{formatFuturePrice(market, priceTicks)}</strong></div>
      <div className="ticket-summary muted"><span>Demo hold</span><strong>{formatUSDC(hold)}</strong></div>
      <div className="ticket-note">{payoffNote}</div>
      <button className={tab === 'buy' ? 'trade-btn long-submit' : 'trade-btn short-submit'} type="button" disabled={!authed || busy || !contracts || !priceTicks} onClick={submit}>
        {busy ? <Loader2 className="spin" size={17} /> : <CircleDollarSign size={17} />} {authed ? `${tab === 'buy' ? 'Buy' : 'Sell'} ${contracts} @ ${formatFuturePrice(market, priceTicks)}` : 'Login to trade'}
      </button>
      <p>Numeric futures are demo USDC-settled contracts.</p>
    </section>
  )
}

function PositionSnapshot({ position, mark, market, authed }) {
  const qty = positionQty(position)
  const avg = positionAvgMicro(position)
  const pnl = livePnlMicro(position, mark, market)
  const scalar = isFutureMarket(market)

  return (
    <section className="position-card">
      <div className="panel-head compact"><h2>Your position</h2><span>{authed ? 'Live mark' : 'Demo login'}</span></div>
      {authed && position ? (
        <div className="position-stat-grid">
          <div><span>Qty</span><strong>{qty}</strong></div>
          <div><span>Avg</span><strong>{scalar ? formatFuturePrice(market, avgPriceTicks(position)) : formatUSDC(avg)}</strong></div>
          <div><span>Mark</span><strong>{scalar ? formatFuturePrice(market, mark) : `${mark || 0}¢`}</strong></div>
          <div><span>Live PnL</span><strong className={pnlClassName(pnl)}>{formatSignedUSDC(pnl)}</strong></div>
        </div>
      ) : (
        <div className="position-empty">No position in this market.</div>
      )}
    </section>
  )
}

function PortfolioPage({ balance, authed, busy, positions, orders, marketPrices, marketByTicker, selectedUser, onDeposit, onRefresh, onExitPosition }) {
  const cash = balance?.cash_micro_usdc ?? balance?.cashMicroUsdc
  const held = balance?.held_micro_usdc ?? balance?.heldMicroUsdc
  const livePnl = positions.reduce((total, position) => total + livePnlMicro(position, marketPrices[position.ticker] || 0, marketByTicker[position.ticker]), 0)

  return (
    <main className="portfolio-page">
      <section className="portfolio-hero">
        <div>
          <p className="portfolio-kicker">{selectedUser.label}</p>
          <h1>Portfolio</h1>
        </div>
        <div className="portfolio-actions">
          <button className="secondary-btn refresh-btn" type="button" onClick={onRefresh}><RefreshCw size={16} /> Refresh</button>
          <button className="fund-btn compact" type="button" disabled={!authed || busy} onClick={onDeposit}><Gift size={16} /> Add $10k demo funds</button>
        </div>
      </section>

      <section className="portfolio-metrics">
        <div>
          <span>Cash</span>
          <strong>{authed ? formatUSDC(cash) : '--'}</strong>
        </div>
        <div>
          <span>Held</span>
          <strong>{authed ? formatUSDC(held) : '--'}</strong>
        </div>
        <div>
          <span>Positions</span>
          <strong>{positions.length}</strong>
        </div>
        <div>
          <span>Live PnL</span>
          <strong className={pnlClassName(livePnl)}>{authed ? formatSignedUSDC(livePnl) : '--'}</strong>
        </div>
        <div>
          <span>Orders</span>
          <strong>{orders.length}</strong>
        </div>
      </section>

      <section className="portfolio-grid-page">
        <div className="portfolio-panel">
          <div className="panel-head"><h2>Positions</h2><span>{positions.length} total</span></div>
          <div className="portfolio-table">
            <div className="portfolio-row positions-header"><span>Ticker</span><span>Net Qty</span><span>Avg</span><span>Mark</span><span>Live PnL</span><span>Realized</span><span>Action</span></div>
            {positions.length ? positions.map((position) => {
              const market = marketByTicker[position.ticker]
              const scalar = isFutureMarket(market)
              const mark = marketPrices[position.ticker] || 0
              const pnl = livePnlMicro(position, mark, market)
              const qty = positionQty(position)
              return (
                <div className="portfolio-row positions-row" key={`${position.user_id || position.userId}-${position.ticker}`}>
                  <span>{position.ticker}</span>
                  <span>{scalar ? futuresPositionLabel(qty) : qty}</span>
                  <span>{scalar ? formatFuturePrice(market, avgPriceTicks(position)) : formatUSDC(positionAvgMicro(position))}</span>
                  <span>{mark ? (scalar ? formatFuturePrice(market, mark) : `${mark}¢`) : '--'}</span>
                  <span className={pnlClassName(pnl)}>{formatSignedUSDC(pnl)}</span>
                  <span>{formatUSDC(position.realized_pnl_micro_usdc ?? position.realizedPnlMicroUsdc)}</span>
                  <span>
                    <button
                      className="exit-position-btn"
                      type="button"
                      disabled={!authed || busy || !qty}
                      onClick={() => onExitPosition(position)}
                    >
                      {busy ? <Loader2 className="spin" size={14} /> : <LogOut size={14} />} Exit
                    </button>
                  </span>
                </div>
              )
            }) : <div className="portfolio-empty">No positions yet.</div>}
          </div>
        </div>

        <div className="portfolio-panel">
          <div className="panel-head"><h2>Orders</h2><span>{orders.length} total</span></div>
          <div className="portfolio-table">
            <div className="portfolio-row header"><span>Ticker</span><span>Trade</span><span>Price</span><span>Status</span></div>
            {orders.length ? orders.map((order) => (
              <PortfolioOrderRow key={order.order_id || order.orderId} order={order} market={marketByTicker[order.ticker]} />
            )) : <div className="portfolio-empty">No open orders.</div>}
          </div>
        </div>
      </section>
    </main>
  )
}

function PortfolioOrderRow({ order, market }) {
  const scalar = isFutureMarket(market)
  const price = order.avg_fill_price_ticks || order.avgFillPriceTicks || order.price_ticks || order.priceTicks
  return (
    <div className="portfolio-row">
      <span>{order.ticker}</span>
      <span>{scalar ? futuresOrderLabel(order) : `${orderActionLabel(order.action)} ${orderSideLabel(order.side)}`}</span>
      <span>{scalar ? formatFuturePrice(market, price) : `${price}¢`}</span>
      <span>{orderStatusLabel(order.status)}</span>
    </div>
  )
}

function OrderBook({ book, market }) {
  const asks = [...(book?.asks || [])].reverse()
  const bids = book?.bids || []
  const scalar = isFutureMarket(market)
  return (
    <section className="book-card">
      <h3><BarChart3 size={18} /> Order book</h3>
      <BookHeader />
      {asks.map((level) => <BookRow key={bookRowKey(level, 'ask')} level={level} type="ask" market={market} />)}
      <div className="spread-row">Spread {scalar ? formatFutureTickDiff(market, spread(book)) : `${spread(book)}¢`}</div>
      {bids.map((level) => <BookRow key={bookRowKey(level, 'bid')} level={level} type="bid" market={market} />)}
    </section>
  )
}

function BookHeader() {
  return <div className="book-header"><span>Price</span><span>Qty</span><span>Orders</span></div>
}

function BookRow({ level, type, market }) {
  const price = level.price_ticks ?? level.priceTicks
  const qty = level.total_qty ?? level.totalQty
  const count = level.order_count ?? level.orderCount
  const width = Math.min(100, Math.max(8, Number(qty) / 3))
  return (
    <div className={`book-row ${type}`}>
      <span>{isFutureMarket(market) ? formatFuturePrice(market, price) : `${price}¢`}</span><span>{qty}</span><span>{count}</span><i style={{ width: `${width}%` }} />
    </div>
  )
}

function RecentTrades({ fills, market }) {
  const latest = [...fills].sort((a, b) => Number(b.global_seq || b.globalSeq || b.seq || 0) - Number(a.global_seq || a.globalSeq || a.seq || 0)).slice(0, 12)
  const scalar = isFutureMarket(market)
  return (
    <section className="trades-card">
      <h3><Activity size={18} /> Recent trades</h3>
      {latest.length ? latest.map((fill) => (
        <div className="trade-line" key={fill.fill_id || fill.fillId || `${fill.global_seq || fill.globalSeq}-${fill.price_ticks || fill.priceTicks}-${fill.count}`}>
          <span>{scalar ? futuresFillLabel(fill) : `${fill.taker_action === 1 || fill.takerAction === 1 ? 'Buy' : 'Sell'} Yes`}</span>
          <strong>{scalar ? formatFuturePrice(market, fill.price_ticks ?? fill.priceTicks) : `${fill.price_ticks ?? fill.priceTicks}¢`}</strong>
          <em>{fill.count} ct</em>
        </div>
      )) : <div className="empty-state">Run the demo simulator to create trades.</div>}
    </section>
  )
}

function avatarText(market) {
  return (market.series_ticker || market.seriesTicker || market.ticker || 'SX').split('-').map((part) => part[0]).join('').slice(0, 2)
}

function impliedPrice(market, fills) {
  const fill = [...fills]
    .filter((item) => item.ticker === market.ticker)
    .sort((a, b) => Number(b.global_seq || b.globalSeq || b.seq || 0) - Number(a.global_seq || a.globalSeq || a.seq || 0))[0]
  return Number(fill?.price_ticks || fill?.priceTicks || (isFutureMarket(market) ? futureFallbackTicks(market) : 50))
}

function isFutureMarket(market) {
  return Number(market?.kind || 0) === SCALAR_KIND
}

function futureMeta(market = {}) {
  const ticker = market.ticker || ''
  const text = `${ticker} ${market.underlying || ''} ${market.question || ''}`.toUpperCase()
  if (text.includes('BTC')) return { divider: 1, decimals: 0, prefix: '$', compactThousands: true }
  if (text.includes('ETH')) return { divider: 1, decimals: 0, prefix: '$', compactThousands: true }
  if (text.includes('GDP') || text.includes('AI')) return { divider: 100, decimals: 2, prefix: '$', suffix: 'T' }
  if (text.includes('NIFTY')) return { divider: 1, decimals: 0, prefix: '' }
  if (text.includes('USDINR') || text.includes('USD/INR')) return { divider: 100, decimals: 2, prefix: '' }
  if (text.includes('CPI') || text.includes('UNEMPLOYMENT') || text.includes('FED')) return { divider: 100, decimals: 2, suffix: '%' }
  return { divider: 100, decimals: 2 }
}

function formatFuturePrice(market, ticks) {
  const meta = futureMeta(market)
  const raw = Number(ticks || 0)
  const value = raw / meta.divider
  if (meta.compactThousands && Math.abs(value) >= 1000) {
    return `${meta.prefix || ''}${(value / 1000).toLocaleString(undefined, { maximumFractionDigits: 1 })}K${meta.suffix || ''}`
  }
  return `${meta.prefix || ''}${value.toLocaleString(undefined, {
    minimumFractionDigits: meta.decimals,
    maximumFractionDigits: meta.decimals,
  })}${meta.suffix || ''}`
}

function formatFutureInput(market, ticks) {
  const meta = futureMeta(market)
  const value = Number(ticks || futureFallbackTicks(market)) / meta.divider
  return value.toFixed(meta.decimals)
}

function parseFutureInput(market, value) {
  const meta = futureMeta(market)
  return Math.round(Number(value || 0) * meta.divider)
}

function formatFutureRange(market) {
  const min = Number(market?.min_price_ticks ?? market?.minPriceTicks ?? market?.lower_bound_ticks ?? market?.lowerBoundTicks ?? 0)
  const max = Number(market?.max_price_ticks ?? market?.maxPriceTicks ?? market?.upper_bound_ticks ?? market?.upperBoundTicks ?? 0)
  if (!min || !max) return 'Demo range'
  return `${formatFuturePrice(market, min)} - ${formatFuturePrice(market, max)}`
}

function formatFutureTick(market) {
  return formatFutureTickDiff(market, Number(market?.tick_size ?? market?.tickSize ?? 1))
}

function formatFutureTickDiff(market, ticks) {
  const meta = futureMeta(market)
  const value = Number(ticks || 0) / meta.divider
  return `${value.toLocaleString(undefined, { maximumFractionDigits: meta.decimals })}${meta.suffix || ''}`
}

function formatFutureMultiplierSpec(market, multiplierMicro) {
  const meta = futureMeta(market)
  const tick = Number(market?.tick_size ?? market?.tickSize ?? 1)
  const multiplier = Number(multiplierMicro || market?.multiplier_micro_usdc || market?.multiplierMicroUsdc || 0)
  if (!multiplier) return 'Demo multiplier'

  const perTick = formatUSDC(multiplier * tick)
  const tickLabel = formatFutureTickDiff(market, tick)
  if (meta.suffix === '%' && meta.divider) {
    const onePointPayout = formatUSDC(multiplier * meta.divider)
    return `${perTick} / ${tickLabel} tick · ${onePointPayout} / 1.00% move`
  }
  return `${perTick} / ${tickLabel} tick`
}

function futureFallbackTicks(market) {
  const min = Number(market?.min_price_ticks ?? market?.minPriceTicks ?? market?.lower_bound_ticks ?? market?.lowerBoundTicks ?? 0)
  const max = Number(market?.max_price_ticks ?? market?.maxPriceTicks ?? market?.upper_bound_ticks ?? market?.upperBoundTicks ?? 0)
  if (min && max) return Math.round((min + max) / 2)
  return 50
}

function clampFutureTicks(market, value) {
  const min = Number(market?.min_price_ticks ?? market?.minPriceTicks ?? market?.lower_bound_ticks ?? market?.lowerBoundTicks ?? 1)
  const max = Number(market?.max_price_ticks ?? market?.maxPriceTicks ?? market?.upper_bound_ticks ?? market?.upperBoundTicks ?? 99)
  const tick = Number(market?.tick_size ?? market?.tickSize ?? 1)
  const rounded = Math.round(Number(value || 0) / tick) * tick
  return Math.max(min, Math.min(max, rounded))
}

function midpoint(bid, ask) {
  return bid && ask ? Math.round((bid + ask) / 2) : 0
}

function computeFutureHoldMicro(market, tab, priceTicks, count) {
  const lower = Number(market?.lower_bound_ticks ?? market?.lowerBoundTicks ?? market?.min_price_ticks ?? market?.minPriceTicks ?? 0)
  const upper = Number(market?.upper_bound_ticks ?? market?.upperBoundTicks ?? market?.max_price_ticks ?? market?.maxPriceTicks ?? 0)
  const multiplier = Number(market?.multiplier_micro_usdc ?? market?.multiplierMicroUsdc ?? 10000)
  const riskTicks = tab === 'sell' ? Math.max(0, upper - priceTicks) : Math.max(0, priceTicks - lower)
  return riskTicks * Math.max(0, Number(count || 0)) * multiplier
}

function avgPriceTicks(position) {
  return Math.round(positionAvgMicro(position) / 10000)
}

function futuresPositionLabel(qty) {
  if (qty > 0) return `LONG ${qty}`
  if (qty < 0) return `SHORT ${Math.abs(qty)}`
  return 'Flat'
}

function futuresOrderLabel(order) {
  return (order.action === 2 || order.action === 'SELL') ? 'Sell / Short' : 'Buy / Long'
}

function futuresFillLabel(fill) {
  return (fill.taker_action === 2 || fill.takerAction === 2 || fill.taker_action === 'SELL' || fill.takerAction === 'SELL') ? 'Sell / Short' : 'Buy / Long'
}

function orderSideLabel(value) {
  if (value === 1 || value === 'YES') return 'Yes'
  if (value === 2 || value === 'NO') return 'No'
  if (value === 3 || value === 'LONG') return 'Long'
  if (value === 4 || value === 'SHORT') return 'Short'
  return 'Yes'
}

function orderActionLabel(value) {
  if (value === 2 || value === 'SELL') return 'Sell'
  return 'Buy'
}

function orderStatusLabel(value) {
  const labels = {
    1: 'Pending',
    2: 'Open',
    3: 'Partial',
    4: 'Filled',
    5: 'Cancelled',
    6: 'Rejected',
    7: 'Expired',
    PENDING: 'Pending',
    OPEN: 'Open',
    PARTIAL: 'Partial',
    FILLED: 'Filled',
    CANCELLED: 'Cancelled',
    REJECTED: 'Rejected',
    EXPIRED: 'Expired',
  }
  return labels[value] || 'Unknown'
}

function activeOrderStatus(value) {
  return value === 2 || value === 3 || value === 'OPEN' || value === 'PARTIAL'
}

function orderRejectMessage(body) {
  const order = body?.order || body
  const code = body?.reject_code || body?.rejectCode || order?.reject_code || order?.rejectCode
  const reason = body?.reject_reason || body?.rejectReason || order?.reject_reason || order?.rejectReason
  const status = order?.status
  if (!code && status !== 6 && status !== 'REJECTED') return ''
  const label = code ? String(code).replaceAll('_', ' ').toLowerCase() : 'order rejected'
  return reason ? `${label}: ${reason}` : label
}

function bookRowKey(level, type) {
  const price = level.price_ticks ?? level.priceTicks
  const qty = level.total_qty ?? level.totalQty
  const count = level.order_count ?? level.orderCount
  return `${type}-${price}-${qty}-${count}`
}

function mergeRecentFills(current, incoming, tickers) {
  const visible = new Set(tickers)
  const byTicker = new Map()
  for (const fill of [...current, ...incoming]) {
    if (!visible.has(fill.ticker)) continue
    const tickerFills = byTicker.get(fill.ticker) || new Map()
    tickerFills.set(fill.fill_id || fill.fillId || `${fill.ticker}-${fillSeq(fill)}-${fill.count}`, fill)
    byTicker.set(fill.ticker, tickerFills)
  }
  return [...byTicker.values()].flatMap((tickerFills) => (
    [...tickerFills.values()]
      .sort((a, b) => fillSeq(a) - fillSeq(b))
      .slice(-300)
  ))
}

function fillSeq(fill) {
  return Number(fill?.global_seq || fill?.globalSeq || fill?.seq || 0)
}

function positionQty(position) {
  return Number(position?.net_qty ?? position?.netQty ?? 0)
}

function positionAvgMicro(position) {
  return Number(position?.avg_cost_micro_usdc ?? position?.avgCostMicroUsdc ?? 0)
}

function maxExitOrderCount(action, priceTicks) {
  const holdTicks = action === 'SELL' ? 100 - Number(priceTicks || 0) : Number(priceTicks || 0)
  return Math.max(1, Math.floor(DEMO_MAX_ORDER_CENTS / Math.max(1, holdTicks)))
}

function livePnlMicro(position, markTicks, market) {
  const qty = positionQty(position)
  if (!qty || !markTicks) return 0
  if (isFutureMarket(market)) {
    const avgTicks = avgPriceTicks(position)
    const multiplier = Number(market?.multiplier_micro_usdc ?? market?.multiplierMicroUsdc ?? 10000)
    if (!avgTicks || !multiplier) return 0
    return (Number(markTicks || 0) - avgTicks) * qty * multiplier
  }
  const avg = positionAvgMicro(position)
  const mark = Number(markTicks || 0) * 10000
  if (!avg || !mark) return 0
  return (mark - avg) * qty
}

function pnlClassName(value) {
  if (value > 0) return 'pnl-positive'
  if (value < 0) return 'pnl-negative'
  return 'pnl-flat'
}

function formatDate(value) {
  const seconds = value?.seconds
  if (!seconds) return 'Demo market'
  return new Date(Number(seconds) * 1000).toLocaleDateString(undefined, { month: 'short', day: 'numeric', year: 'numeric' })
}

function formatTime(value) {
  const date = new Date(value)
  if (Number.isNaN(date.getTime())) return '--'
  return date.toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit', second: '2-digit' })
}

function formatUSDC(value = 0) {
  return `$${(Number(value || 0) / 1_000_000).toLocaleString(undefined, { maximumFractionDigits: 2 })}`
}

function formatSignedUSDC(value = 0) {
  const dollars = Number(value || 0) / 1_000_000
  const sign = dollars > 0 ? '+' : dollars < 0 ? '-' : ''
  return `${sign}$${Math.abs(dollars).toLocaleString(undefined, { maximumFractionDigits: 2 })}`
}

function spread(book) {
  const bid = Number(book?.bids?.[0]?.price_ticks || book?.bids?.[0]?.priceTicks || 0)
  const ask = Number(book?.asks?.[0]?.price_ticks || book?.asks?.[0]?.priceTicks || 0)
  if (!bid || !ask) return '--'
  return Math.max(0, ask - bid)
}

function buildChartPoints(fills, fallback, market) {
  const source = fills.length ? fills.slice(-34).map((fill) => Number(fill.price_ticks || fill.priceTicks || fallback)) : []
  const scalar = isFutureMarket(market)
  const min = scalar ? Number(market?.min_price_ticks ?? market?.minPriceTicks ?? 0) : 0
  const max = scalar ? Number(market?.max_price_ticks ?? market?.maxPriceTicks ?? Math.max(fallback * 1.2, fallback + 10)) : 100
  const wave = scalar ? Math.max(1, Math.round((max - min) * 0.015)) : 7
  const values = source.length ? source : Array.from({ length: 28 }, (_, i) => Math.max(min + 1, Math.min(max - 1, fallback + Math.sin(i / 3) * wave + i * (scalar ? 0.2 : 0.3))))
  const points = values.map((value, index) => {
    const x = (index / Math.max(1, values.length - 1)) * 720
    const y = 248 - ((value - min) / (max - min)) * 220
    return [x, y]
  })
  const line = points.map(([x, y], index) => `${index === 0 ? 'M' : 'L'} ${x.toFixed(2)} ${y.toFixed(2)}`).join(' ')
  return { line, area: line }
}

export default App
