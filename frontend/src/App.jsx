/* eslint-disable react-hooks/set-state-in-effect */
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import {
  Activity,
  ArrowLeft,
  BarChart3,
  Bookmark,
  CircleDollarSign,
  ChevronDown,
  Clock3,
  Gift,
  Hexagon,
  Link2,
  Loader2,
  LogOut,
  RefreshCw,
  Share2,
  SlidersHorizontal,
  Search,
  UserRound,
} from 'lucide-react'
import { dispose as disposeKlineChart, init as initKlineChart, registerStyles } from 'klinecharts'
import { contractsCatalog } from './contractsCatalog'
import { buildKlineBars, futureMeta, futureConfigurationIssue, futureLimitPriceIssue, parseFutureInput } from './futures'
import MatrixBackdrop from '../test/MatrixBackdrop'
import MidPriceChart from '../test/chart desin 1/MidPriceChart'
import { BinaryCard } from '../test/binary card design/BinaryMarkets'
import { BINARY_CSS } from '../test/binary card design/binary-core'
import { CARD_CSS } from '../test/futures card design/cards-core'
import './App.css'

registerStyles('sarvexKlineTheme', {
  grid: {
    show: true,
    horizontal: { color: '#292b32', style: 'dashed', dashedValue: [2, 2] },
    vertical: { color: '#24262c', style: 'dashed', dashedValue: [2, 2] },
  },
  candle: {
    type: 'candle_solid',
    bar: {
      upColor: '#167f72',
      upBorderColor: '#3fd0c2',
      upWickColor: '#3fd0c2',
      downColor: '#bb3f72',
      downBorderColor: '#ef72a6',
      downWickColor: '#ef72a6',
      noChangeColor: '#8f8d97',
      noChangeBorderColor: '#8f8d97',
      noChangeWickColor: '#8f8d97',
    },
  },
  xAxis: { axisLine: { color: '#383a42' }, tickText: { color: '#777681' } },
  yAxis: { axisLine: { color: '#383a42' }, tickText: { color: '#aaa9b1' } },
  separator: { color: '#292b32' },
})

const API_BASE = import.meta.env.VITE_API_BASE_URL || '/api'
const DEMO_MAX_ORDER_CENTS = 10000
const LIVE_TRADE_REFRESH_MS = 1500
const LIVE_PAGE_REFRESH_MS = 6000
const MARKET_LIST_REFRESH_MS = 30000
const FILL_PAGE_LIMIT = 40
const SCALAR_KIND = 2
const MARKET_SECTIONS = ['All', 'Economics', 'Finance', 'Crypto', 'Commodities', 'Elections', 'Climate', 'Geopolitics / Shipping']
const HIDDEN_DEMO_MARKET_TICKERS = new Set([
  'DEMO-INDIA-GDP-Q2-26-7PCT',
  'RBI-JUN26-CUT25',
])
const DEMO_CONTRACT_ASSUMPTIONS = {
  'SX-PRESNOMD-28-{CAND}': 'Will Gavin Newsom be the 2028 Democratic presidential nominee?',
  'SX-TASI-26OCT29-ATM': 'TASI above 11,500 on 29 Oct 2026?',
  'SX-N225-26OCT30-ATM': 'Nikkei above 42,000 on 30 Oct 2026?',
  'SX-TTF-26OCT30-ATM': 'EU gas (TTF) above EUR 35 end-Oct?',
}
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
  const [markets, setMarkets] = useState([])
  const [futures, setFutures] = useState([])
  const [selectedTicker, setSelectedTicker] = useState('')
  const [searchQuery, setSearchQuery] = useState('')
  const [orderbook, setOrderbook] = useState(null)
  const [fills, setFills] = useState([])
  const [balance, setBalance] = useState(null)
  const [positions, setPositions] = useState([])
  const [orders, setOrders] = useState([])
  const [history, setHistory] = useState([])
  const [bookMarks, setBookMarks] = useState({})
  const [pinnedTickers, setPinnedTickers] = useState(() => {
    try {
      const saved = JSON.parse(localStorage.getItem('sarvex_pinned_markets') || '[]')
      return Array.isArray(saved) ? saved.filter((ticker) => typeof ticker === 'string') : []
    } catch {
      return []
    }
  })
  const [loading, setLoading] = useState(true)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const [activeView, setActiveView] = useState(() => viewFromPath(window.location.pathname))
  const fillCursorRef = useRef({})
  const lastMarketListRefreshRef = useRef(0)
  const marketsRef = useRef([])
  const futuresRef = useRef([])
  const selectedTickerRef = useRef('')
  const activeViewRef = useRef(activeView)

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

  const togglePinnedTicker = useCallback((ticker) => {
    setPinnedTickers((current) => {
      const next = current.includes(ticker)
        ? current.filter((item) => item !== ticker)
        : [...current, ticker]
      localStorage.setItem('sarvex_pinned_markets', JSON.stringify(next))
      return next
    })
  }, [])

  const api = useCallback(
    async (path, options = {}) => {
      const { auth = true, headers: optionHeaders = {}, ...fetchOptions } = options
      const headers = { ...optionHeaders }
      if (fetchOptions.body && !Object.keys(headers).some((key) => key.toLowerCase() === 'content-type')) {
        headers['Content-Type'] = 'application/json'
      }
      if (auth && token) headers.Authorization = `Bearer ${token}`
      const response = await fetch(`${API_BASE}${path}`, { ...fetchOptions, headers })
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
    async () => {
      setBusy(true)
      setError('')
      try {
        const nextUserId = DEMO_USERS[0].id
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
      } catch (err) {
        setError(err.message)
      } finally {
        setBusy(false)
      }
    },
    [],
  )

  const logout = useCallback(() => {
    localStorage.removeItem('sarvex_token')
    localStorage.removeItem('sarvex_user_id')
    setToken('')
    setBalance(null)
    setPositions([])
    setOrders([])
    setHistory([])
  }, [])

  const fetchMarketFills = useCallback(async (ticker) => {
    let cursor = fillCursorRef.current[ticker] || ''
    const query = new URLSearchParams({ limit: String(FILL_PAGE_LIMIT) })
    if (cursor) query.set('cursor', cursor)
    const body = await api(`/v1/markets/${ticker}/fills?${query.toString()}`, { auth: false })
    const incoming = body?.fills || []

    const maxSeq = incoming.reduce((max, fill) => Math.max(max, fillSeq(fill)), Number(fillCursorRef.current[ticker] || 0))
    if (maxSeq > 0) fillCursorRef.current[ticker] = String(maxSeq)
    return incoming
  }, [api])

  const refreshPublic = useCallback(async ({ refreshMarkets = false } = {}) => {
    setError('')
    let nextMarkets = marketsRef.current
    let nextFutures = futuresRef.current
    const shouldRefreshMarkets = refreshMarkets || !nextMarkets.length || !nextFutures.length || Date.now() - lastMarketListRefreshRef.current > MARKET_LIST_REFRESH_MS
    if (shouldRefreshMarkets) {
      // The refdata API permits up to 200 contracts; load the complete MVP
      // catalog so cards are not incorrectly treated as catalog-only.
      const marketBody = await api('/v1/markets?state=OPEN&limit=200', { auth: false })
      const liveContracts = marketBody?.contracts || []
      const catalogContracts = contractsCatalog.map(catalogMarket)
      const liveByTicker = new Map(liveContracts.map((market) => [market.ticker, market]))
      const mergedCatalog = catalogContracts
        .map((market) => ({ ...market, ...(liveByTicker.get(market.ticker) || {}), catalogOnly: !liveByTicker.has(market.ticker) }))
      const catalogTickers = new Set(catalogContracts.map((market) => market.ticker))
      const liveOnlyContracts = liveContracts
        .filter((market) => !catalogTickers.has(market.ticker))
        .map((market) => ({ ...market, ...workbookMetadata(market), catalogOnly: false }))
      const mergedContracts = [...mergedCatalog, ...liveOnlyContracts]
        .map(resolveDemoContractQuestion)
      const nonSportsContracts = mergedContracts.filter((market) => !isSportsMarket(market))
      nextMarkets = nonSportsContracts
        .filter((market) => !isFutureMarket(market) && !HIDDEN_DEMO_MARKET_TICKERS.has(market.ticker))
      nextFutures = nonSportsContracts.filter((market) => isFutureMarket(market))
      marketsRef.current = nextMarkets
      futuresRef.current = nextFutures
      lastMarketListRefreshRef.current = Date.now()
      setMarkets(nextMarkets)
      setFutures(nextFutures)
    }
    const visibleContracts = [...nextMarkets, ...nextFutures]
    const liveTickers = new Set(visibleContracts.filter((market) => !market.catalogOnly).map((market) => market.ticker))
    const ticker = liveTickers.has(selectedTickerRef.current) ? selectedTickerRef.current : [...liveTickers][0]
    const liveVisibleContracts = visibleContracts.filter((market) => liveTickers.has(market.ticker))
    const fillContracts = ticker
      ? liveVisibleContracts.filter((market) => market.ticker === ticker)
      : liveVisibleContracts.slice(0, 12)
    const fillsByMarket = await Promise.all(fillContracts.map((market) => fetchMarketFills(market.ticker).catch(() => [])))
    const incomingFills = fillsByMarket.flat()
    setFills((current) => mergeRecentFills(current, incomingFills, visibleContracts.map((market) => market.ticker)))
    if (!ticker) return
    setOrderbook(await api(`/v1/markets/${ticker}/orderbook?depth=12`, { auth: false }))
  }, [api, fetchMarketFills])

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
      const book = await api(`/v1/markets/${ticker}/orderbook?depth=1`, { auth: false })
      const bid = Number(book?.bids?.[0]?.price_ticks || book?.bids?.[0]?.priceTicks || 0)
      const ask = Number(book?.asks?.[0]?.price_ticks || book?.asks?.[0]?.priceTicks || 0)
      const mark = midpoint(bid, ask) || ask || bid || 0
      return [ticker, mark]
    }))
    setBookMarks(Object.fromEntries(entries.filter(([, mark]) => mark > 0)))
  }, [api])

  const refreshPrivate = useCallback(async () => {
    if (!token) return
    const [balanceBody, positionsBody, ordersBody, historyBody] = await Promise.all([
      api('/v1/account/balance'),
      api('/v1/positions?include_closed=false'),
      api('/v1/orders?limit=500'),
      api('/v1/account/history?limit=500').catch(() => ({ entries: [] })),
    ])
    const nextPositions = positionsBody?.positions || []
    const nextOrders = ordersBody?.orders || []
    setBalance(balanceBody)
    setPositions(nextPositions)
    setOrders(nextOrders)
    setHistory(historyBody?.entries || [])
    refreshBookMarks(nextPositions, nextOrders).catch(() => {})
  }, [api, refreshBookMarks, token])

  const refreshAll = useCallback(async () => {
    setLoading(true)
    try {
      await refreshPublic({ refreshMarkets: true })
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
    marketsRef.current = markets
  }, [markets])

  useEffect(() => {
    futuresRef.current = futures
  }, [futures])

  useEffect(() => {
    selectedTickerRef.current = selectedTicker
    if (selectedTicker) refreshPublic().catch((err) => setError(err.message))
  }, [refreshPublic, selectedTicker])

  useEffect(() => {
    activeViewRef.current = activeView
  }, [activeView])

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

  const selectedUser = DEMO_USERS[0]
  const selectedFills = selectedMarket ? fills.filter((fill) => fill.ticker === selectedMarket.ticker) : []

  return (
    <div className="sarvex-shell">
      <TopNav
        selectedUser={selectedUser}
        token={token}
        busy={busy}
        onLogin={login}
        onLogout={logout}
        onMarkets={handleMarketsNav}
        searchQuery={searchQuery}
        onSearchChange={setSearchQuery}
        searchMarkets={[...markets, ...futures]}
        onSelectMarket={(ticker) => {
          setSearchQuery('')
          handleMarketSelect(ticker)
        }}
        onTerminal={() => {
          const ticker = selectedTickerRef.current || marketsRef.current[0]?.ticker || futuresRef.current[0]?.ticker
          if (ticker) handleMarketSelect(ticker)
          else handleMarketsNav()
        }}
        onNavigateView={handleViewNav}
        activeView={activeView}
      />

      {error && <div className="notice error">{error}</div>}

      {activeView === 'trade' && selectedMarket && selectedIsFuture ? (
        <FutureDetail
          market={selectedMarket}
          watchlist={futures}
          watchlistFills={fills}
          onSelect={handleMarketSelect}
          pinnedTickers={pinnedTickers}
          onTogglePin={togglePinnedTicker}
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
          watchlist={markets}
          onSelect={handleMarketSelect}
          pinnedTickers={pinnedTickers}
          onTogglePin={togglePinnedTicker}
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
          history={history}
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
          searchQuery={searchQuery}
        />
      ) : (
        <MarketDashboard
          loading={loading}
          markets={markets}
          fills={fills}
          onSelect={handleMarketSelect}
          onRefresh={refreshAll}
          searchQuery={searchQuery}
        />
      )}
    </div>
  )
}

function TopNav({ selectedUser, token, busy, onLogin, onLogout, onMarkets, onTerminal, onNavigateView, activeView, searchQuery, onSearchChange, searchMarkets, onSelectMarket }) {
  const normalizedQuery = searchQuery.trim().toLowerCase()
  const searchResults = normalizedQuery
    ? searchMarkets.filter((market) => marketMatchesSearch(market, normalizedQuery)).slice(0, 6)
    : []

  const handleSearchKeyDown = (event) => {
    if (event.key === 'Escape') onSearchChange('')
    if (event.key === 'Enter' && searchResults[0]) onSelectMarket(searchResults[0].ticker)
  }

  return (
    <header className="topbar">
      <button className="brand" type="button" onClick={onMarkets}>
        <span className="brand-mark-new" aria-hidden="true"><Hexagon size={25} strokeWidth={1.8} /></span>
        <span>Sarvaex</span>
      </button>
      <div className="topbar-center">
        <div className="global-search-wrap">
          <label className="global-search"><Search size={15} /><input value={searchQuery} onChange={(event) => onSearchChange(event.target.value)} onKeyDown={handleSearchKeyDown} placeholder="Search" aria-label="Search markets" /><kbd>/</kbd></label>
          {normalizedQuery ? (
            <div className="search-results" role="listbox" aria-label="Search results">
              {searchResults.length ? searchResults.map((market) => (
                <button className="search-result" type="button" key={market.ticker} onClick={() => onSelectMarket(market.ticker)}>
                  <span>{market.question || market.underlying || market.ticker}</span>
                  <small>{market.ticker}</small>
                </button>
              )) : <div className="search-empty">No markets found</div>}
            </div>
          ) : null}
        </div>
        <nav className="main-nav">
          <button className={activeView === 'markets' ? 'nav-link active' : 'nav-link'} type="button" onClick={onMarkets}>Discover</button>
          <button className={activeView === 'futures' ? 'nav-link active' : 'nav-link'} type="button" onClick={() => onNavigateView('futures')}>Futures</button>
          <button className={activeView === 'trade' ? 'nav-link active' : 'nav-link'} type="button" onClick={onTerminal}>Terminal</button>
          <button className={activeView === 'portfolio' ? 'nav-link active' : 'nav-link'} type="button" onClick={() => onNavigateView('portfolio')}>Portfolio</button>
          <button className={activeView === 'health' ? 'nav-link active' : 'nav-link'} type="button" onClick={() => onNavigateView('health')}>Health</button>
        </nav>
      </div>
      <div className="user-cluster">
        {token ? (
          <>
            <span className="account-icon" title={`${selectedUser.label} account`} aria-label={`${selectedUser.label} account`}><UserRound size={17} /></span>
            <button className="logout-btn" type="button" onClick={onLogout}><LogOut size={15} /> Log out</button>
          </>
        ) : (
          <button className="demo-login-btn" type="button" onClick={onLogin} disabled={busy}>
            {busy ? <Loader2 className="spin" size={15} /> : null}Log in demo
          </button>
        )}
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
          <p className="portfolio-kicker">Sarvaex system status</p>
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

function MarketDashboard({ loading, markets, fills, onSelect, onRefresh, searchQuery }) {
  const [section, setSection] = useState('All')
  const rows = markets
    .filter((market) => section === 'All' || contractSection(market) === section)
    .filter((market) => marketMatchesSearch(market, searchQuery))
  return (
    <main className="dashboard-page">
      <style>{BINARY_CSS}</style>
      <nav className="category-nav" aria-label="Market categories">
        {MARKET_SECTIONS.map((item) => <button className={section === item ? 'category-link active' : 'category-link'} type="button" key={item} onClick={() => setSection(item)}>{item}</button>)}
      </nav>
      <section className="dashboard-hero">
        <div className="promo-banner">
          <MatrixBackdrop
            focus="34% 55%"
            options={{ market: 'SARVAEX LIVE MARKETS' }}
          />
          <div className="matrix-hero-copy"><span>Powered by Sarvaex</span><strong>Trade What Happens Next</strong></div>
        </div>
        <aside className="live-panel">
          <div className="live-panel-head"><span><i /> Live markets</span><span>1 / 11 <ChevronDown size={13} /></span></div>
          {markets.slice(0, 5).map((market, index) => <button type="button" className="live-market" key={market.ticker} onClick={() => onSelect(market.ticker)}><span className={`live-avatar avatar-${index}`}>{avatarText(market)}</span><span><small>{contractSection(market)} · {market.catalogOnly ? 'Planned' : 'Live'}</small><b>{market.question || market.underlying || market.ticker}</b></span><strong>{Math.max(1, Math.min(99, impliedPrice(market, fills)))}%</strong></button>)}
        </aside>
      </section>

      {loading ? (
        <div className="loading-panel"><Loader2 className="spin" /> Loading Sarvaex markets...</div>
      ) : rows.length ? (
        <section className="market-grid">
          {rows.map((market, index) => (
            <MarketCard
              key={market.ticker}
              market={market}
              fills={fills}
              index={index}
              section={section}
              onClick={() => onSelect(market.ticker)}
            />
          ))}
        </section>
      ) : <div className="loading-panel">No markets match your search.</div>}
    </main>
  )
}

function MarketCard({ market, fills, section, onClick }) {
  const price = impliedPrice(market, fills)
  const cardMarket = {
    id: market.ticker,
    question: cardMarketTitle(market),
    category: section === 'All' ? binaryCardCategory(market) : section,
    yes: price,
    change: 0,
    yesAsk: price,
    noAsk: 100 - price,
    settle: marketSettlement(market),
    traded: !market.catalogOnly,
  }
  return <BinaryCard market={cardMarket} onBuy={onClick} onOpen={onClick} />
}

function FuturesDashboard({ loading, futures, fills, onSelect, searchQuery }) {
  const [section, setSection] = useState('All')
  const rows = futures
    .filter((market) => section === 'All' || contractSection(market) === section)
    .filter((market) => marketMatchesSearch(market, searchQuery))
  return (
    <main className="dashboard-page">
      <style>{CARD_CSS}</style>
      <nav className="category-nav" aria-label="Futures categories">
        {MARKET_SECTIONS.map((item) => <button className={section === item ? 'category-link active' : 'category-link'} type="button" key={item} onClick={() => setSection(item)}>{item}</button>)}
      </nav>

      {loading ? (
        <div className="loading-panel"><Loader2 className="spin" /> Loading Sarvaex futures...</div>
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
        <div className="loading-panel">{searchQuery.trim() ? 'No futures match your search.' : 'No numeric futures are open yet.'}</div>
      )}
    </main>
  )
}

function FutureCard({ market, fills, onClick }) {
  const price = impliedPrice(market, fills)
  const min = Number(market.min_price_ticks ?? market.minPriceTicks ?? 0)
  const max = Number(market.max_price_ticks ?? market.maxPriceTicks ?? Math.max(price, 1))
  const position = max > min ? Math.max(0, Math.min(100, ((price - min) / (max - min)) * 100)) : 50
  return (
    <article className="smc" role="button" tabIndex="0" onClick={onClick} onKeyDown={(event) => { if (event.key === 'Enter' || event.key === ' ') { event.preventDefault(); onClick() } }}>
      <div className="smc-top">
        <div className="smc-title"><span className="smc-dot" style={{ '--c': scalarCardColor(market) }} aria-hidden="true" /><span>{cardMarketTitle(market)}</span></div>
        <span className="smc-exp">{formatDate(market.close_at || market.closeAt || market.expected_resolution_at)}</span>
      </div>
      <div className="smc-sub">{market.underlying || market.question || market.ticker}</div>
      <div className="smc-val"><b>{formatFuturePrice(market, price)}</b></div>
      <div className="smc-track"><div className="smc-fill" style={{ width: `${position}%` }} /><div className="smc-mark" style={{ left: `${position}%` }} /></div>
      <div className="smc-ends"><span>{formatFuturePrice(market, min)}</span><span>{formatFuturePrice(market, max)}</span></div>
    </article>
  )
}

function TerminalWatchlist({ markets, selectedTicker, onSelect, fills = [], pinnedTickers = [] }) {
  const pinned = new Set(pinnedTickers)
  const orderedMarkets = [...markets].sort((a, b) => Number(pinned.has(b.ticker)) - Number(pinned.has(a.ticker)))

  return (
    <aside className="terminal-watchlist">
      <div className="terminal-watchlist-head"><span>Markets</span><span><RefreshCw size={12} /></span></div>
      <div className="terminal-watchlist-list">
        {orderedMarkets.slice(0, 14).map((item, index) => <button className={`${item.ticker === selectedTicker ? 'watch-item active' : 'watch-item'}${pinned.has(item.ticker) ? ' pinned' : ''}`} type="button" key={item.ticker} onClick={() => onSelect(item.ticker)}>
          <span className={`watch-icon watch-${index % 5}`}>{avatarText(item)}</span>
          <span><b>{pinned.has(item.ticker) && <span className="watch-pin" aria-label="Pinned">•</span>}{item.question || item.ticker}</b>{isFutureMarket(item)
            ? <small>{fills.some((fill) => fill.ticker === item.ticker) ? formatFuturePrice(item, impliedPrice(item, fills)) : '--'}</small>
            : <small>{Math.max(1, Math.min(99, impliedPrice(item, fills)))}¢ <em>{100 - Math.max(1, Math.min(99, impliedPrice(item, fills)))}¢</em></small>}</span>
        </button>)}
      </div>
      {!markets.length && <div className="watch-empty">No markets loaded</div>}
    </aside>
  )
}

function MarketDetail({ market, watchlist, onSelect, orderbook, fills, position, authed, busy, onBack, onTrade, pinnedTickers, onTogglePin }) {
  const bestBid = Number(orderbook?.bids?.[0]?.price_ticks || orderbook?.bids?.[0]?.priceTicks || 0)
  const bestAsk = Number(orderbook?.asks?.[0]?.price_ticks || orderbook?.asks?.[0]?.priceTicks || 0)
  const last = Number(fills?.[fills.length - 1]?.price_ticks || fills?.[fills.length - 1]?.priceTicks || bestAsk || bestBid || 50)

  return (
    <main className="detail-page terminal-page">
      <TerminalWatchlist markets={watchlist} selectedTicker={market.ticker} onSelect={onSelect} pinnedTickers={pinnedTickers} />
      <section className="market-main">
        <button className="back-btn" type="button" onClick={onBack}><ArrowLeft size={16} /> All markets</button>
        <div className="detail-heading">
          <div className="market-avatar xl">{avatarText(market)}</div>
          <div>
            <p className="crumb">{market.series_ticker || market.seriesTicker || 'Sarvaex'} · {market.kind === 2 ? 'Scalar Future' : 'Binary Contract'}</p>
            <h1>{market.question || market.underlying || market.ticker}</h1>
          </div>
          <div className="heading-actions"><Share2 size={18} /><Link2 size={18} /><button className={pinnedTickers?.includes(market.ticker) ? 'pin-btn active' : 'pin-btn'} type="button" title={pinnedTickers?.includes(market.ticker) ? 'Unpin market' : 'Pin market'} aria-label={pinnedTickers?.includes(market.ticker) ? 'Unpin market' : 'Pin market'} onClick={() => onTogglePin?.(market.ticker)}><Bookmark size={18} /></button></div>
        </div>

        <div className="metric-row">
          <span>Last traded: <strong>{last}¢</strong></span>
          <span><Clock3 size={15} /> {formatDate(market.close_at || market.closeAt || market.expected_resolution_at)}</span>
          <span><Activity size={15} /> {fills.length} recent fills</span>
        </div>

        <section className="binary-chart-shell">
          <BinaryCleanChart fills={fills} bestBid={bestBid} bestAsk={bestAsk} market={market} fallback={last} />
        </section>

        <OracleDetails market={market} />

        <section className="contracts-table">
          <ContractRow title="Yes" price={bestAsk || last} side="yes" onClick={() => document.querySelector('#trade-ticket')?.scrollIntoView({ behavior: 'smooth' })} />
          <ContractRow title="No" price={100 - (bestBid || last)} side="no" onClick={() => document.querySelector('#trade-ticket')?.scrollIntoView({ behavior: 'smooth' })} />
        </section>

        <section className="lower-grid">
          <RecentTrades fills={fills} market={market} />
        </section>
      </section>

      <aside className="trade-side">
        <OrderBook book={orderbook} market={market} />
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

function BinaryCleanChart({ fills, bestBid, bestAsk, market, fallback }) {
  const getData = useCallback(async (range) => {
    const now = Date.now()
    const rangeMs = { '1H': 3600000, '6H': 21600000, '1D': 86400000, '1W': 604800000, '1M': 2592000000, ALL: Infinity }[range] || 86400000
    const source = fills
      .map((fill, index) => {
        const price = Number(fill.price_ticks || fill.priceTicks || fallback)
        return { fill, index, price: Math.max(1, Math.min(99, price)), t: fillTimestamp(fill, index, fills.length, now) }
      })
      .filter((item) => item.t >= now - rangeMs)
      .sort((a, b) => a.t - b.t)
    const mid = Math.max(1, Math.min(99, midpoint(bestBid, bestAsk) || fallback || 50))
    const points = source.length >= 3
      ? source.map((item) => ({ t: item.t, bid: Math.max(1, item.price - 1), ask: Math.min(99, item.price + 1) }))
      : Array.from({ length: 3 }, (_, index) => ({ t: now - (2 - index) * 3600000, bid: Math.max(1, mid - 1), ask: Math.min(99, mid + 1) }))
    return {
      points,
      fills: source.map((item) => ({ t: item.t, price: item.price, qty: Number(item.fill.count || item.fill.qty || 1), side: item.fill.side || 'buy' })),
      events: [],
      last: source.at(-1)?.price || mid,
      closeLabel: formatDate(market.close_at || market.closeAt || market.expected_resolution_at),
    }
  }, [bestAsk, bestBid, fallback, fills, market])

  return <MidPriceChart key={`${market.ticker}-${fills.length}`} getData={getData} initialRange="1D" height={300} />
}

function FutureDetail({ market, watchlist, watchlistFills, onSelect, orderbook, fills, position, authed, busy, onBack, onTrade, pinnedTickers, onTogglePin }) {
  const [chartPeriod, setChartPeriod] = useState('1h')
  const bestBid = Number(orderbook?.bids?.[0]?.price_ticks || orderbook?.bids?.[0]?.priceTicks || 0)
  const bestAsk = Number(orderbook?.asks?.[0]?.price_ticks || orderbook?.asks?.[0]?.priceTicks || 0)
  const last = Number(fills?.[fills.length - 1]?.price_ticks || fills?.[fills.length - 1]?.priceTicks || midpoint(bestBid, bestAsk) || futureFallbackTicks(market))
  const multiplier = Number(market.multiplier_micro_usdc ?? market.multiplierMicroUsdc ?? 0)

  return (
    <main className="detail-page terminal-page">
      <TerminalWatchlist markets={watchlist} fills={watchlistFills} selectedTicker={market.ticker} onSelect={onSelect} pinnedTickers={pinnedTickers} />
      <section className="market-main">
        <button className="back-btn" type="button" onClick={onBack}><ArrowLeft size={16} /> All futures</button>
        <div className="detail-heading">
          <div className="market-avatar xl">{avatarText(market)}</div>
          <div>
            <p className="crumb">{market.series_ticker || market.seriesTicker || 'Sarvaex'} · Numeric Future</p>
            <h1>{market.question || market.underlying || market.ticker}</h1>
          </div>
          <div className="heading-actions"><Share2 size={18} /><Link2 size={18} /><button className={pinnedTickers?.includes(market.ticker) ? 'pin-btn active' : 'pin-btn'} type="button" title={pinnedTickers?.includes(market.ticker) ? 'Unpin market' : 'Pin market'} aria-label={pinnedTickers?.includes(market.ticker) ? 'Unpin market' : 'Pin market'} onClick={() => onTogglePin?.(market.ticker)}><Bookmark size={18} /></button></div>
        </div>

        <div className="metric-row">
          <span>Current price: <strong>{formatFuturePrice(market, last)}</strong></span>
          <span><Clock3 size={15} /> {formatDate(market.close_at || market.closeAt || market.expected_resolution_at)}</span>
          <span><Activity size={15} /> {fills.length} recent fills</span>
          <span>Tick: <strong>{formatFutureTick(market)}</strong></span>
        </div>

        <section className="chart-card">
          <div className="future-chart-topline">
            <div className="future-chart-title">
              <BarChart3 size={15} />
              <strong>Market price</strong>
              <span>Candlestick</span>
            </div>
            <div className="future-chart-periods" aria-label="Chart timeframe">
              {['1m', '5m', '1h', '1D'].map((period) => (
                <button className={chartPeriod === period ? 'active' : ''} key={period} type="button" onClick={() => setChartPeriod(period)}>{period}</button>
              ))}
              <button type="button" aria-label="Chart settings"><SlidersHorizontal size={14} /></button>
            </div>
          </div>
          <div className="future-chart-meta">
            <strong>{formatFuturePrice(market, last)}</strong>
            <span>{fills.length} recent fills</span>
            <span>Linear USDC-settled demo future</span>
          </div>
          <KlineFutureChart fills={fills} market={market} period={chartPeriod} />
        </section>

        <OracleDetails market={market} />

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
          <RecentTrades fills={fills} market={market} />
        </section>
      </section>

      <aside className="trade-side">
        <OrderBook book={orderbook} market={market} />
        <FutureTradeTicket
          key={market.ticker}
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

function chartPeriodConfig(period) {
  if (period === '1m') return { span: 1, type: 'minute' }
  if (period === '5m') return { span: 5, type: 'minute' }
  if (period === '1D') return { span: 1, type: 'day' }
  return { span: 1, type: 'hour' }
}

function KlineFutureChart({ fills, market, period }) {
  const containerRef = useRef(null)
  const chartRef = useRef(null)
  const bars = useMemo(() => buildKlineBars(fills, market, period), [fills, market, period])
  const pricePrecision = futureMeta(market).decimals
  const barsRef = useRef(bars)

  useEffect(() => {
    barsRef.current = bars
  }, [bars])

  useEffect(() => {
    const container = containerRef.current
    if (!container) return undefined

    const chart = initKlineChart(container, {
      styles: 'sarvexKlineTheme',
      timezone: 'America/New_York',
      layout: { yAxis: { position: 'right' } },
    })
    if (!chart) return undefined

    chart.setDataLoader({
      getBars: ({ callback }) => callback(barsRef.current),
    })
    chart.setSymbol({ ticker: market.ticker, pricePrecision, volumePrecision: 0 })
    chart.setPeriod(chartPeriodConfig(period))
    chart.resetData()
    chart.resize()
    chartRef.current = chart

    const resizeObserver = typeof ResizeObserver === 'undefined'
      ? null
      : new ResizeObserver(() => chart.resize())
    resizeObserver?.observe(container)

    return () => {
      resizeObserver?.disconnect()
      disposeKlineChart(chart)
      chartRef.current = null
    }
  }, [market.ticker, period, pricePrecision])

  useEffect(() => {
    const chart = chartRef.current
    if (!chart) return
    chart.setDataLoader({
      getBars: ({ callback }) => callback(barsRef.current),
    })
    chart.resetData()
  }, [bars])

  return <div className="future-chart">
    <div className="kline-chart-container" ref={containerRef} role="img" aria-label="Futures candlestick price chart" />
    {!bars.length && <div className="future-chart-empty">No trades yet</div>}
  </div>
}

function OracleDetails({ market }) {
  const source = market?.settlement_source || market?.settlementSource || 'Not published'
  const policy = market?.oracle_policy || market?.oraclePolicy || 'Not published'
  const underlying = market?.underlying || (isFutureMarket(market) ? 'Numeric value defined by the contract' : 'Binary outcome defined by the contract')
  const closeAt = market?.close_at || market?.closeAt
  const resolutionAt = market?.expected_resolution_at || market?.expectedResolutionAt

  return (
    <section className="oracle-details">
      <div className="panel-head compact">
        <div className="oracle-title"><Activity size={16} /><h2>Oracle & settlement</h2></div>
        <span>{market?.catalogOnly ? 'Awaiting refdata' : 'Configured'}</span>
      </div>
      <div className="oracle-grid">
        <div className="oracle-item oracle-wide">
          <span>Underlying / observation</span>
          <strong>{underlying}</strong>
        </div>
        <div className="oracle-item">
          <span>Settlement source</span>
          <strong>{source}</strong>
        </div>
        <div className="oracle-item">
          <span>Oracle policy</span>
          <strong>{policy}</strong>
        </div>
        <div className="oracle-item">
          <span>Settlement rule</span>
          <strong>{formatSettlementRule(market?.settlement_rule || market?.settlementRule)}</strong>
        </div>
        <div className="oracle-item">
          <span>Trading closes</span>
          <strong>{formatDate(closeAt)}</strong>
        </div>
        <div className="oracle-item">
          <span>Expected resolution</span>
          <strong>{formatDate(resolutionAt)}</strong>
        </div>
      </div>
    </section>
  )
}

function TradeTicket({ market, bestBid, bestAsk, authed, busy, onTrade }) {
  const [tab, setTab] = useState('buy')
  const [outcome, setOutcome] = useState('YES')
  const [orderType, setOrderType] = useState('market')
  const [limitInput, setLimitInput] = useState(String(bestAsk || bestBid || 50))
  const [amount, setAmount] = useState('10')
  const action = outcome === 'YES'
    ? (tab === 'buy' ? 'BUY' : 'SELL')
    : (tab === 'buy' ? 'SELL' : 'BUY')
  const referenceYes = action === 'BUY' ? (bestAsk || bestBid || 50) : (bestBid || bestAsk || 50)
  const referenceOutcomePrice = outcome === 'YES' ? referenceYes : 100 - referenceYes
  const limitOutcomePrice = clampBinaryPrice(Number(limitInput || referenceOutcomePrice))
  const displayedPrice = orderType === 'market' ? clampBinaryPrice(referenceOutcomePrice) : limitOutcomePrice
  const priceTicks = orderType === 'market' ? 0 : clampBinaryPrice(outcome === 'YES' ? limitOutcomePrice : 100 - limitOutcomePrice)
  const spend = Math.max(0, Number(amount || 0))
  const requestedShares = spend > 0 ? Math.floor(spend / (displayedPrice / 100)) : 0
  const maxShares = Math.max(1, Math.floor(DEMO_MAX_ORDER_CENTS / displayedPrice))
  const shares = requestedShares > 0 ? Math.max(1, Math.min(requestedShares, maxShares)) : 0
  const capped = requestedShares > maxShares
  const estimatedSpend = shares * displayedPrice / 100

  const updateAmount = (value) => {
    const nextAmount = Number(value || 0)
    setAmount(String(Math.min(100, Math.max(0, nextAmount))))
  }

  const suggestedLimit = (nextOutcome = outcome, nextTab = tab) => {
    const nextAction = nextOutcome === 'YES'
      ? (nextTab === 'buy' ? 'BUY' : 'SELL')
      : (nextTab === 'buy' ? 'SELL' : 'BUY')
    const yesPrice = nextAction === 'BUY' ? (bestAsk || bestBid || 50) : (bestBid || bestAsk || 50)
    return String(clampBinaryPrice(nextOutcome === 'YES' ? yesPrice : 100 - yesPrice))
  }

  const switchTab = (nextTab) => {
    setTab(nextTab)
    setLimitInput(suggestedLimit(outcome, nextTab))
  }

  const switchOutcome = (nextOutcome) => {
    setOutcome(nextOutcome)
    setLimitInput(suggestedLimit(nextOutcome, tab))
  }

  const submit = () => {
    if (!shares) return
    const id = `fe-${Date.now()}-${Math.random().toString(16).slice(2)}`
    onTrade({
      client_order_id: id,
      ticker: market.ticker,
      side: 'YES',
      action,
      order_type: orderType === 'market' ? 'MARKET' : 'LIMIT',
      price_ticks: priceTicks,
      count: shares,
      tif: orderType === 'market' ? 'IOC' : 'GTC',
    })
  }

  return (
    <section className="ticket" id="trade-ticket">
      <div className="ticket-market"><div className="market-avatar small">{avatarText(market)}</div><span>{market.ticker}</span></div>
      <div className="ticket-tabs">
        <button className={tab === 'buy' ? 'active' : ''} type="button" onClick={() => switchTab('buy')}>Buy</button>
        <button className={tab === 'sell' ? 'active' : ''} type="button" onClick={() => switchTab('sell')}>Sell</button>
      </div>
      <div className="order-type-toggle">
        <button className={orderType === 'market' ? 'active' : ''} type="button" onClick={() => setOrderType('market')}>Market</button>
        <button className={orderType === 'limit' ? 'active' : ''} type="button" onClick={() => setOrderType('limit')}>Limit</button>
      </div>
      <div className="outcome-toggle">
        <button className={outcome === 'YES' ? 'yes active' : 'yes'} type="button" onClick={() => switchOutcome('YES')}>Yes {clampBinaryPrice(referenceYes)}¢</button>
        <button className={outcome === 'NO' ? 'no active' : 'no'} type="button" onClick={() => switchOutcome('NO')}>No {clampBinaryPrice(100 - referenceYes)}¢</button>
      </div>
      {orderType === 'limit' ? (
        <label className="amount-input ticket-price-input">
          <span>Limit price</span>
          <input value={limitInput} onChange={(event) => setLimitInput(event.target.value.replace(/[^\d.]/g, ''))} inputMode="decimal" />
        </label>
      ) : (
        <div className="ticket-summary market-estimate"><span>Market estimate</span><strong>{displayedPrice}¢</strong></div>
      )}
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
        {busy ? <Loader2 className="spin" size={17} /> : <CircleDollarSign size={17} />} {authed ? `${orderType === 'market' ? 'Market' : 'Limit'} trade` : 'Login to trade'}
      </button>
      <p>By trading, you agree to Sarvaex demo terms.</p>
    </section>
  )
}

function FutureTradeTicket({ market, bestBid, bestAsk, mark, authed, busy, onTrade }) {
  const [tab, setTab] = useState('buy')
  const [orderType, setOrderType] = useState('market')
  const [priceInput, setPriceInput] = useState(() => formatFutureInput(market, bestAsk || bestBid || mark))
  const [size, setSize] = useState('5')
  const parsedPrice = parseFutureInput(market, priceInput)
  const priceTicks = Number.isFinite(parsedPrice) ? parsedPrice : 0
  const marketPriceTicks = tab === 'buy' ? (bestAsk || mark || bestBid || futureFallbackTicks(market)) : (bestBid || mark || bestAsk || futureFallbackTicks(market))
  const executionTicks = orderType === 'market' ? marketSweepTicks(market, tab === 'buy' ? 'BUY' : 'SELL') : priceTicks
  const contracts = Math.max(0, Math.floor(Number(size || 0)))
  const hold = computeFutureHoldMicro(market, tab, executionTicks, contracts)
  const orderIssue = futureConfigurationIssue(market)
    || (orderType === 'limit' ? futureLimitPriceIssue(market, parsedPrice) : '')
    || (!Number.isSafeInteger(contracts) || contracts <= 0 ? 'Enter a positive whole number of contracts.' : '')
    || (contracts > Number(market.max_order_size ?? market.maxOrderSize ?? Infinity) ? 'Quantity exceeds the contract order limit.' : '')
  const payoffNote = tab === 'buy'
    ? 'Final payoff: (final value - entry) x contracts x multiplier.'
    : 'Final payoff: (entry - final value) x contracts x multiplier.'
  const switchSide = (nextTab) => {
    setTab(nextTab)
    const nextPrice = nextTab === 'buy' ? (bestAsk || mark || bestBid) : (bestBid || mark || bestAsk)
    if (nextPrice) setPriceInput(formatFutureInput(market, nextPrice))
  }

  const submit = () => {
    if (orderIssue) return
    const id = `fut-${Date.now()}-${Math.random().toString(16).slice(2)}`
    onTrade({
      client_order_id: id,
      ticker: market.ticker,
      side: 'LONG',
      action: tab === 'buy' ? 'BUY' : 'SELL',
      order_type: orderType === 'market' ? 'MARKET' : 'LIMIT',
      price_ticks: orderType === 'market' ? 0 : priceTicks,
      count: contracts,
      tif: orderType === 'market' ? 'IOC' : 'GTC',
    })
  }

  return (
    <section className="ticket futures-ticket" id="trade-ticket">
      <div className="ticket-market"><div className="market-avatar small">{avatarText(market)}</div><span>{market.ticker}</span></div>
      <div className="ticket-tabs">
        <button className={tab === 'buy' ? 'active long-tab' : 'long-tab'} type="button" onClick={() => switchSide('buy')}>Buy / Long</button>
        <button className={tab === 'sell' ? 'active short-tab' : 'short-tab'} type="button" onClick={() => switchSide('sell')}>Sell / Short</button>
      </div>
      <div className="order-type-toggle">
        <button className={orderType === 'market' ? 'active' : ''} type="button" onClick={() => setOrderType('market')}>Market</button>
        <button className={orderType === 'limit' ? 'active' : ''} type="button" onClick={() => setOrderType('limit')}>Limit</button>
      </div>
      {orderType === 'limit' ? (
        <label className="amount-input future-price-input">
          <span>Entry price{futureMeta(market).suffix ? ` (${futureMeta(market).suffix})` : ''}</span>
          <input value={priceInput} onChange={(event) => setPriceInput(event.target.value)} inputMode="decimal" />
        </label>
      ) : (
        <div className="ticket-summary market-estimate"><span>Market estimate</span><strong>{formatFuturePrice(market, marketPriceTicks)}</strong></div>
      )}
      <label className="amount-input future-size-input">
        <span>Contracts</span>
        <input value={size} onChange={(event) => { if (/^\d*$/.test(event.target.value)) setSize(event.target.value) }} inputMode="numeric" />
      </label>
      <div className="quick-amounts">
        {[1, 5, 10, 25].map((value) => <button type="button" key={value} onClick={() => setSize(String(Math.max(0, Number(size || 0)) + value))}>+{value}</button>)}
      </div>
      <div className="ticket-summary"><span>{orderType === 'market' ? 'Est. entry' : 'Entry'}</span><strong>{formatFuturePrice(market, orderType === 'market' ? marketPriceTicks : priceTicks)}</strong></div>
      <div className="ticket-summary muted"><span>{orderType === 'market' ? 'Max. collateral' : 'Required collateral'}</span><strong>{formatUSDC(hold)}</strong></div>
      <div className="ticket-note">{payoffNote}</div>
      {orderIssue && <div className="ticket-validation" role="alert">{orderIssue}</div>}
      <button className={tab === 'buy' ? 'trade-btn long-submit' : 'trade-btn short-submit'} type="button" disabled={!authed || busy || Boolean(orderIssue)} onClick={submit}>
        {busy ? <Loader2 className="spin" size={17} /> : <CircleDollarSign size={17} />} {authed ? `${orderType === 'market' ? 'Market' : 'Limit'} ${tab === 'buy' ? 'buy' : 'sell'} ${contracts}` : 'Login to trade'}
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

function PortfolioPage({ balance, authed, busy, positions, orders, history, marketPrices, marketByTicker, selectedUser, onDeposit, onRefresh, onExitPosition }) {
  const cash = balance?.cash_micro_usdc ?? balance?.cashMicroUsdc
  const held = balance?.held_micro_usdc ?? balance?.heldMicroUsdc
  const total = balance?.total_micro_usdc ?? balance?.totalMicroUsdc ?? Number(cash || 0) + Number(held || 0)
  const openPositions = positions.filter((position) => positionQty(position) !== 0)
  const livePnl = positions.reduce((total, position) => total + livePnlMicro(position, marketPrices[position.ticker] || 0, marketByTicker[position.ticker]), 0)
  const realizedPnl = positions.reduce((total, position) => total + Number(position.realized_pnl_micro_usdc ?? position.realizedPnlMicroUsdc ?? 0), 0)
  const openValue = openPositions.reduce((total, position) => total + positionValueMicro(position, marketPrices[position.ticker] || 0, marketByTicker[position.ticker]), 0)
  const resolvedPositions = positions.filter((position) => Number(position.realized_pnl_micro_usdc ?? position.realizedPnlMicroUsdc ?? 0) !== 0)
  const winningPositions = resolvedPositions.filter((position) => Number(position.realized_pnl_micro_usdc ?? position.realizedPnlMicroUsdc) > 0).length
  const winRate = resolvedPositions.length ? `${Math.round((winningPositions / resolvedPositions.length) * 100)}%` : '--'

  return (
    <main className="portfolio-page">
      <section className="portfolio-hero">
        <h1>Portfolio</h1>
        <div className="portfolio-actions">
          <button className="secondary-btn refresh-btn" type="button" onClick={onRefresh}><RefreshCw size={16} /> Refresh</button>
          <button className="fund-btn compact" type="button" disabled={!authed || busy} onClick={onDeposit}><Gift size={16} /> Add $10k demo funds</button>
        </div>
      </section>

      <section className="portfolio-account-strip">
        <div className="portfolio-account-name"><strong>{selectedUser.label}</strong><span>Demo account</span></div>
        <PortfolioAccountMetric label="Portfolio" value={authed ? formatUSDC(total) : '--'} />
        <PortfolioAccountMetric label="Positions" value={authed ? formatUSDC(held) : '--'} />
        <PortfolioAccountMetric label="Live PnL" value={authed ? formatSignedUSDC(livePnl) : '--'} tone={pnlClassName(livePnl)} />
        <PortfolioAccountMetric label="Orders" value={orders.length} />
        <div className="portfolio-account-tools"><button type="button" title="Refresh portfolio" onClick={onRefresh}><RefreshCw size={15} /></button><button type="button" title="Add demo funds" onClick={onDeposit} disabled={!authed || busy}><Gift size={15} /></button></div>
      </section>

      <section className="portfolio-workspace">
        <aside className="portfolio-metrics-rail">
          <div className="workspace-title">Metrics</div>
          <div className="portfolio-winrate"><strong>{authed ? winRate : '--'}</strong><span>Win Rate</span><em>{authed ? formatSignedUSDC(livePnl) : '--'} <small>Live PnL</small></em></div>
          <PortfolioRailMetric label="Realized PnL" value={authed ? formatSignedUSDC(realizedPnl) : '--'} tone={pnlClassName(realizedPnl)} />
          <PortfolioRailMetric label="Unrealized PnL" value={authed ? formatSignedUSDC(livePnl) : '--'} tone={pnlClassName(livePnl)} />
          <PortfolioRailMetric label="Open Positions" value={openPositions.length} />
          <PortfolioRailMetric label="At Risk" value={authed ? formatUSDC(held) : '--'} />
          <PortfolioRailMetric label="Open Value" value={authed ? formatUSDC(openValue) : '--'} />
          <PortfolioRailMetric label="Volume" value={orders.reduce((total, order) => total + Number(order.filled_count ?? order.filledCount ?? 0), 0)} />
        </aside>

        <section className="portfolio-calendar-panel">
          <div className="workspace-tabs"><button className="active" type="button">Calendar</button><button type="button">Chart</button><div className="workspace-tabs-spacer" /><button className="active" type="button">PnL</button><button type="button">Volume</button></div>
          <PortfolioCalendar history={history} />
        </section>
      </section>

      <section className="portfolio-grid-page portfolio-data-panels">
        <div className="portfolio-panel">
          <div className="panel-head"><h2>Positions</h2><span>{openPositions.length} total</span></div>
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

function PortfolioAccountMetric({ label, value, tone = '' }) {
  return <div className="portfolio-account-metric"><span>{label}</span><strong className={tone}>{value}</strong></div>
}

function PortfolioRailMetric({ label, value, tone = '' }) {
  return <div className="portfolio-rail-metric"><span>{label}</span><strong className={tone}>{value}</strong></div>
}

function PortfolioCalendar({ history }) {
  const today = new Date()
  const monthStart = new Date(today.getFullYear(), today.getMonth(), 1)
  const daysInMonth = new Date(today.getFullYear(), today.getMonth() + 1, 0).getDate()
  const offset = (monthStart.getDay() + 6) % 7
  const cells = Array.from({ length: offset + daysInMonth }, (_, index) => index < offset ? null : index - offset + 1)
  while (cells.length % 7) cells.push(null)
  const activityByDay = history.reduce((activity, entry) => {
    if (!String(entry.account_code || '').endsWith(':CASH')) return activity
    const date = new Date(entry.posted_at || 0)
    if (Number.isNaN(date.getTime()) || date.getFullYear() !== today.getFullYear() || date.getMonth() !== today.getMonth()) return activity
    const day = date.getDate()
    const amount = Number(entry.amount_micro_usdc || 0) * (entry.direction === 'CR' ? 1 : -1)
    activity[day] = (activity[day] || 0) + amount
    return activity
  }, {})
  return (
    <div className="portfolio-calendar">
      <div className="calendar-toolbar"><button type="button">‹</button><strong>{today.toLocaleDateString(undefined, { month: 'short', year: 'numeric' })}</strong><button type="button">›</button><span>All⌄</span></div>
      <div className="calendar-weekdays">{['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'].map((day) => <span key={day}>{day}</span>)}</div>
      <div className="calendar-grid">{cells.map((day, index) => <div className={day === today.getDate() ? 'calendar-day today' : 'calendar-day'} key={`${day || 'empty'}-${index}`}>{day && <><span>{day}</span>{activityByDay[day] !== undefined && <small className={activityByDay[day] >= 0 ? 'activity-positive' : 'activity-negative'}>{formatSignedUSDC(activityByDay[day])}</small>}</>}</div>)}</div>
      <div className="calendar-legend"><span><i className="loss" /> Outflow</span><span><i className="profit" /> Inflow</span><span><i className="best" /> Activity</span></div>
    </div>
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

function MarketImage({ market, index = 0, size = '' }) {
  const imageIndex = marketImageIndex(market, index)
  return (
    <div className={`market-avatar market-image ${size}`}>
      <img src={`/market-art-${imageIndex}.svg`} alt="" aria-hidden="true" />
    </div>
  )
}

function marketImageIndex(market, index = 0) {
  const value = String(market?.ticker || market?.underlying || market?.question || index)
  const hash = [...value].reduce((sum, character) => sum + character.charCodeAt(0), 0)
  return (hash + index) % 5 + 1
}

function catalogMarket(contract) {
  const future = contract.leg === 'Futures'
  return {
    ticker: contract.ticker,
    kind: future ? SCALAR_KIND : 1,
    question: future ? '' : DEMO_CONTRACT_ASSUMPTIONS[contract.ticker] || contract.title,
    underlying: future ? contract.title : '',
    category: contract.category,
    subcategory: contract.subcategory,
    region: contract.region,
    section: contract.section,
    pair_id: contract.pairId,
    launch_priority: contract.priority,
    catalogOnly: true,
  }
}

function resolveDemoContractQuestion(market) {
  const assumedQuestion = DEMO_CONTRACT_ASSUMPTIONS[market?.ticker]
  if (!assumedQuestion) return market

  const question = String(market?.question || '')
  return /<K>|<candidate>/i.test(question) || !question
    ? { ...market, question: assumedQuestion }
    : market
}

function cardMarketTitle(market) {
  const title = market?.question || market?.underlying || market?.ticker || 'Untitled market'
  return String(title).replace(/\s*\([^)]*\)/g, '').replace(/\s{2,}/g, ' ').trim()
}

function contractSection(market) {
  const category = String(market?.category || workbookMetadata(market).category || '').trim()
  const normalized = category.toLowerCase()
  const identity = `${market?.ticker || ''} ${market?.question || ''} ${market?.underlying || ''}`.toLowerCase()
  if (normalized.startsWith('economics')) return 'Economics'
  if (normalized.startsWith('finance') || normalized.startsWith('fx') || normalized.startsWith('local equities')) return 'Finance'
  if (normalized.startsWith('crypto') || /\b(crypto|bitcoin|btc|ethereum|eth)\b/.test(identity)) return 'Crypto'
  if (normalized.startsWith('commodities') || normalized.startsWith('energy')) return 'Commodities'
  if (normalized.startsWith('elections')) return 'Elections'
  if (normalized.startsWith('climate')) return 'Climate'
  if (normalized.startsWith('geopolitics')) return 'Geopolitics / Shipping'
  return category || 'Other'
}

function binaryCardCategory(market) {
  const section = contractSection(market)
  return ['Economics', 'Finance', 'Crypto', 'Commodities', 'Elections', 'Climate', 'Geopolitics / Shipping'].includes(section)
    ? section
    : 'Other'
}

function scalarCardColor(market) {
  return ({
    Economics: '#8b7ff0',
    Finance: '#2bb3a0',
    Crypto: '#d9c04a',
    Commodities: '#d0496a',
    Climate: '#5b94d6',
    'Geopolitics / Shipping': '#4fb6d6',
  })[contractSection(market)] || '#6d5ce8'
}

function isSportsMarket(market) {
  return String(market?.category || workbookMetadata(market).category || '').startsWith('Sports')
}

function workbookMetadata(market) {
  const rule = market?.settlement_rule || market?.settlementRule || {}
  return {
    category: rule.category || '',
    subcategory: rule.subcategory || '',
    region: rule.region || '',
    pair_id: rule.pair_id || '',
    launch_priority: rule.priority || '',
  }
}

function marketMatchesSearch(market, query) {
  const needle = String(query || '').trim().toLowerCase()
  if (!needle) return true
  return [
    market?.ticker,
    market?.question,
    market?.underlying,
    market?.category,
    market?.subcategory,
    market?.region,
  ].some((value) => String(value || '').toLowerCase().includes(needle))
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

function marketSweepTicks(market, action) {
  const min = Number(market?.min_price_ticks ?? market?.minPriceTicks ?? market?.lower_bound_ticks ?? market?.lowerBoundTicks ?? 1)
  const max = Number(market?.max_price_ticks ?? market?.maxPriceTicks ?? market?.upper_bound_ticks ?? market?.upperBoundTicks ?? 99)
  return action === 'SELL' ? min : max
}

function clampBinaryPrice(value) {
  return Math.max(1, Math.min(99, Math.round(Number(value || 50))))
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

function positionValueMicro(position, markTicks, market) {
  const qty = Math.abs(positionQty(position))
  const mark = Number(markTicks || 0)
  if (!qty || !mark) return 0
  if (isFutureMarket(market)) {
    return qty * mark * Number(market?.multiplier_micro_usdc ?? market?.multiplierMicroUsdc ?? 10000)
  }
  return qty * mark * 10000
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

function formatSettlementRule(value) {
  if (!value) return 'Not published'
  let rule = value
  if (typeof value === 'string') {
    try {
      rule = JSON.parse(value)
    } catch {
      return value
    }
  }
  if (rule?.type === 'categorical_equals') {
    const values = Array.isArray(rule.yes_values) ? rule.yes_values.join(', ') : 'YES'
    return `Binary: ${values}`
  }
  if (rule?.type === 'scalar_numeric') return 'Scalar numeric'
  return rule?.type || 'Configured rule'
}

function formatTime(value) {
  const date = new Date(value)
  if (Number.isNaN(date.getTime())) return '--'
  return date.toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit', second: '2-digit' })
}

function fillTimestamp(fill, index, total, now = Date.now()) {
  const raw = fill?.ts || fill?.timestamp || fill?.created_at || fill?.createdAt
  if (raw && typeof raw === 'object' && raw.seconds != null) {
    return Number(raw.seconds) * 1000 + Math.round(Number(raw.nanos || 0) / 1e6)
  }
  const numeric = Number(raw)
  if (Number.isFinite(numeric) && numeric > 0) return numeric < 1e12 ? numeric * 1000 : numeric
  const parsed = raw ? Date.parse(raw) : NaN
  return Number.isFinite(parsed) ? parsed : now - (total - index) * 3600000
}

function marketSettlement(market) {
  const question = String(market?.question || market?.underlying || '')
  const month = '(Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec)(?:ember|uary|rch|il|e|y|ne|ly|ust|tember|ober)?'
  const dayFirst = question.match(new RegExp(`(?:\\b|on |after |before |by |at )([0-9]{1,2})(?:\\s*[–-]\\s*[0-9]{1,2})?\\s+${month}\\s+([0-9]{4})`, 'i'))
  const monthFirst = question.match(new RegExp(`\\b${month}\\s+([0-9]{1,2}),?\\s+([0-9]{4})`, 'i'))
  if (dayFirst) return dateOnlyUtc(dayFirst[3], dayFirst[2], dayFirst[1])
  if (monthFirst) return dateOnlyUtc(monthFirst[3], monthFirst[1], monthFirst[2])
  const raw = market?.close_at || market?.closeAt || market?.expected_resolution_at || market?.expectedResolutionAt
  if (!raw) return 'recurring'
  if (typeof raw === 'object' && raw.seconds != null) {
    const ms = Number(raw.seconds) * 1000 + Math.round(Number(raw.nanos || 0) / 1e6)
    return Number.isFinite(ms) ? new Date(ms).toISOString().slice(0, 10) : 'recurring'
  }
  const numeric = Number(raw)
  if (Number.isFinite(numeric) && numeric > 0) {
    const ms = numeric < 1e12 ? numeric * 1000 : numeric
    return new Date(ms).toISOString().slice(0, 10)
  }
  const parsed = Date.parse(raw)
  return Number.isFinite(parsed) ? new Date(parsed).toISOString().slice(0, 10) : 'recurring'
}

function dateOnlyUtc(year, month, day) {
  const date = new Date(`${month} ${day}, ${year} UTC`)
  return Number.isNaN(date.getTime()) ? 'recurring' : date.toISOString().slice(0, 10)
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

function chartY(value, min, max) {
  return 248 - ((value - min) / Math.max(1, max - min)) * 220
}

function buildChartPoints(fills, fallback, market) {
  const source = fills.length ? fills.slice(-34).map((fill) => Number(fill.price_ticks || fill.priceTicks || fallback)) : []
  const scalar = isFutureMarket(market)
  const min = scalar ? Number(market?.min_price_ticks ?? market?.minPriceTicks ?? 0) : 0
  const max = scalar ? Number(market?.max_price_ticks ?? market?.maxPriceTicks ?? Math.max(fallback * 1.2, fallback + 10)) : 100
  const wave = scalar ? Math.max(1, Math.round((max - min) * 0.02)) : 7
  let seed = [...String(market?.ticker || 'sarvex')].reduce((sum, char) => ((sum * 31) + char.charCodeAt(0)) >>> 0, 7)
  const values = source.length ? source : Array.from({ length: 34 }, (_, i) => {
    seed = (seed * 1664525 + 1013904223) >>> 0
    const jitter = ((seed % 1000) / 1000 - 0.5) * wave * 1.8
    const swing = Math.sin(i / 2.7) * wave * 1.15 + Math.sin(i / 6.5) * wave * 1.7
    const drift = i * (scalar ? 0.12 : 0.22)
    return Math.max(min + 1, Math.min(max - 1, fallback + swing + jitter + drift))
  })
  const points = values.map((value, index) => {
    const x = (index / Math.max(1, values.length - 1)) * 720
    const y = chartY(value, min, max)
    return [x, y]
  })
  const line = points.map(([x, y], index) => `${index === 0 ? 'M' : 'L'} ${x.toFixed(2)} ${y.toFixed(2)}`).join(' ')
  return { line, area: line, points: points.map(([x, y], index) => ({ x, y, value: values[index] })) }
}

export default App
