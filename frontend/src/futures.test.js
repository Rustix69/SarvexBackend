import test from 'node:test'
import assert from 'node:assert/strict'
import { buildKlineBars, futureMeta, futureConfigurationIssue, futureLimitPriceIssue, parseFutureInput } from './futures.js'

const market = { ticker: 'SXF-USCPIYOY-26SEP', min_price_ticks: 100, max_price_ticks: 800, tick_size: 1 }
const minute = Date.parse('2026-09-16T10:00:00Z')
const fill = (id, price, seconds, count = 1) => ({ fill_id: id, ticker: market.ticker, price_ticks: price, ts: minute + seconds * 1000, count })

test('CPI input and candle values use the same percentage-point scale', () => {
  assert.equal(parseFutureInput(market, '3.40'), 340)
  assert.equal(buildKlineBars([fill('1', 340, 1)], market)[0].close, 3.4)
  assert.equal(futureMeta(market).suffix, '%')
})

test('minute candles use ordered fills, actual volume, gaps, and deduplicated IDs', () => {
  const fills = [fill('3', 339, 30, 3), fill('1', 340, 1, 2), fill('2', 342, 20, 4), fill('4', 345, 122, 7), fill('2', 342, 20, 4)]
  assert.deepEqual(buildKlineBars(fills, market), [
    { timestamp: minute, open: 3.4, high: 3.42, low: 3.39, close: 3.39, volume: 9 },
    { timestamp: minute + 120_000, open: 3.45, high: 3.45, low: 3.45, close: 3.45, volume: 7 },
  ])
})

test('equal timestamps use the exchange sequence without truncating int64 strings', () => {
  const fills = [
    { ...fill('2', 342, 1), global_seq: '9007199254740993' },
    { ...fill('1', 340, 1), global_seq: '9007199254740992' },
  ]
  const [bar] = buildKlineBars(fills, market)
  assert.equal(bar.open, 3.4)
  assert.equal(bar.close, 3.42)
})

test('missing or malformed fills never produce invented candles', () => {
  assert.deepEqual(buildKlineBars([], market), [])
  assert.deepEqual(buildKlineBars([
    { price_ticks: 340, count: 1 },
    { ...fill('1', 340, 1), ticker: 'OTHER' },
    fill('2', NaN, 2), fill('3', 0, 3), fill('4', 340, 4, -1),
  ], market), [])
})

test('protobuf camelCase fills and timestamps are supported', () => {
  const [bar] = buildKlineBars([{ fillId: '1', priceTicks: '340', count: '2', ts: { seconds: String(minute / 1000), nanos: 5 } }], market)
  assert.equal(bar.timestamp, minute)
  assert.equal(bar.close, 3.4)
  assert.equal(bar.volume, 2)
})

test('limit prices are rejected rather than silently clamped or rounded', () => {
  assert.equal(futureLimitPriceIssue(market, parseFutureInput(market, '3.40')), '')
  for (const value of ['', 'abc', 'Infinity', '3.401', '9007199254740993']) assert.ok(Number.isNaN(parseFutureInput(market, value)))
  for (const value of ['0', '-1', '9']) assert.notEqual(futureLimitPriceIssue(market, parseFutureInput(market, value)), '')
  assert.notEqual(futureLimitPriceIssue({ ...market, tick_size: 5 }, 341), '')
})

test('placeholder workbook contract configuration is identified without flagging valid contracts', () => {
  const placeholder = { ...market, series_ticker: 'XLSX-STANDARD', lower_bound_ticks: 1, upper_bound_ticks: 1000000, multiplier_micro_usdc: 1000000 }
  assert.match(futureConfigurationIssue(placeholder), /placeholder/)
  assert.equal(futureConfigurationIssue({ ...placeholder, upper_bound_ticks: 800 }), '')
  assert.match(futureConfigurationIssue({ catalogOnly: true }), /not listed/)
})

test('rainfall and strait contracts do not accidentally match AI market capitalization', () => {
  for (const underlying of ['Rainfall', 'Strait of Hormuz']) assert.notEqual(futureMeta({ underlying }).suffix, 'T')
  assert.equal(futureMeta({ ticker: 'FUT-AI-MCAP-DEC26-SCALAR' }).suffix, 'T')
})
