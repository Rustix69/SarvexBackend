// These fallbacks describe the existing demo tick encoding, not workbook terms.
// Changing a contract's encoding requires a coordinated refdata/book migration.
export function futureMeta(market = {}) {
  const ticker = String(market.ticker || '').toUpperCase()
  const text = `${ticker} ${market.underlying || ''} ${market.question || ''}`.toUpperCase()
  if (/BTC|BITCOIN|ETHEREUM|\bETH\b|SXF-ETHD/.test(text)) return { divider: 1, decimals: 0, prefix: '$', compactThousands: true }
  if (/^FUT-(INDIA-GDP|AI-MCAP)-/.test(ticker)) return { divider: 100, decimals: 2, prefix: '$', suffix: 'T' }
  if (/NIFTY|NDX|SPX|N225/.test(text)) return { divider: 1, decimals: 0 }
  if (/USDINR|USD\/INR/.test(text)) return { divider: 100, decimals: 2 }
  if (/EURUSD|EUR\/USD/.test(text)) return { divider: 10000, decimals: 4 }
  if (/AAAGAS|GASOLINE/.test(text)) return { divider: 1000, decimals: 3, prefix: '$' }
  if (/WTI|BRENT/.test(text)) return { divider: 100, decimals: 2, prefix: '$' }
  if (/XAU|GOLD/.test(text)) return { divider: 10, decimals: 1, prefix: '$' }
  if (/HIGHNY|HIGHDXB|HIGHTYO/.test(text)) return { divider: 10, decimals: 1 }
  if (/BTC|BITCOIN/.test(text)) return { divider: 1, decimals: 0, prefix: '$', compactThousands: true }
  if (/ETH|ETHEREUM/.test(text)) return { divider: 1, decimals: 0, prefix: '$', compactThousands: true }
  if (/FFUB/.test(text)) return { divider: 200, decimals: 3, suffix: '%' }
  if (/CPI|UNEMPLOYMENT|FED|RBIREPO|UST10Y|BOJRATE|LPR1Y|ECBDFR|EZHICP|CBUAE/.test(text)) return { divider: 100, decimals: 2, suffix: '%' }
  return { divider: 100, decimals: 2 }
}

export function futureConfigurationIssue(market = {}) {
  const series = market.series_ticker ?? market.seriesTicker ?? ''
  const lower = Number(market.lower_bound_ticks ?? market.lowerBoundTicks)
  const upper = Number(market.upper_bound_ticks ?? market.upperBoundTicks)
  const multiplier = Number(market.multiplier_micro_usdc ?? market.multiplierMicroUsdc)
  if (series.startsWith('XLSX-') && lower === 1 && upper === 1_000_000 && multiplier === 1_000_000) {
    return 'Trading unavailable: this contract still has placeholder price bounds and collateral settings.'
  }
  if (market.catalogOnly) return 'This contract is not listed for trading yet.'
  return ''
}

export function parseFutureInput(market, input) {
  const text = String(input).trim()
  if (!/^-?\d+(?:\.\d+)?$/.test(text)) return NaN
  const scaled = Number(text) * futureMeta(market).divider
  const ticks = Math.round(scaled)
  // Tolerate floating point representation noise, never round a submitted price.
  if (!Number.isSafeInteger(ticks) || Math.abs(scaled - ticks) > 1e-7) return NaN
  return ticks
}

export function futureLimitPriceIssue(market, ticks) {
  const min = Number(market.min_price_ticks ?? market.minPriceTicks)
  const max = Number(market.max_price_ticks ?? market.maxPriceTicks)
  const step = Number(market.tick_size ?? market.tickSize)
  if (!Number.isSafeInteger(ticks)) return 'Enter a valid price with the contract precision.'
  if (ticks <= 0) return 'This demo currently supports positive futures prices only.'
  if (!Number.isSafeInteger(min) || !Number.isSafeInteger(max) || !Number.isSafeInteger(step) || step <= 0) return 'Contract price configuration is unavailable.'
  if (ticks < min || ticks > max) return 'Entry price is outside the contract range.'
  if (ticks % step !== 0) return 'Entry price must align with the contract tick size.'
  return ''
}

function timestampMs(value) {
  if (value && typeof value === 'object' && value.seconds !== undefined) {
    return Number(value.seconds) * 1000 + Number(value.nanos || 0) / 1_000_000
  }
  if (typeof value === 'number') return value > 1e12 ? value : value * 1000
  return typeof value === 'string' ? Date.parse(value) : NaN
}

export function buildKlineBars(fills, market, period = '1h') {
  const divider = futureMeta(market).divider
  const bucketMs = period === '1m'
    ? 60_000
    : period === '5m'
      ? 5 * 60_000
      : period === '1D'
        ? 24 * 60 * 60_000
        : 60 * 60_000
  const seen = new Set()
  const trades = fills.flatMap((fill) => {
    if (fill.ticker && fill.ticker !== market.ticker) return []
    const id = fill.fill_id ?? fill.fillId
    if (id && seen.has(id)) return []
    const priceTicks = Number(fill.price_ticks ?? fill.priceTicks)
    const timestamp = timestampMs(fill.ts ?? fill.timestamp)
    const volume = Number(fill.count)
    if (!Number.isSafeInteger(priceTicks) || priceTicks <= 0 || !Number.isFinite(timestamp) || timestamp <= 0 || !Number.isSafeInteger(volume) || volume <= 0) return []
    if (id) seen.add(id)
    return [{ price: priceTicks / divider, timestamp, volume, seq: BigInt(fill.global_seq ?? fill.globalSeq ?? fill.seq ?? 0) }]
  }).sort((a, b) => a.timestamp - b.timestamp || (a.seq < b.seq ? -1 : a.seq > b.seq ? 1 : 0))

  const bars = []
  for (const trade of trades) {
    const timestamp = Math.floor(trade.timestamp / bucketMs) * bucketMs
    const last = bars[bars.length - 1]
    if (!last || last.timestamp !== timestamp) {
      bars.push({ timestamp, open: trade.price, high: trade.price, low: trade.price, close: trade.price, volume: trade.volume })
    } else {
      last.high = Math.max(last.high, trade.price)
      last.low = Math.min(last.low, trade.price)
      last.close = trade.price
      last.volume += trade.volume
    }
  }
  return bars
}
