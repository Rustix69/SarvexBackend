use anyhow::{Context, Result};
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use std::{env, time::Duration};

const MIN_BOTS: usize = 20;
const MAX_BOTS: usize = 50;

#[derive(Clone, Debug)]
struct Config {
    rest_url: String,
    bot_count: usize,
    funding_usdc: i64,
    book_levels: usize,
    interval: Duration,
    rounds: u64,
    tickers: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct MarketList {
    #[serde(default)]
    contracts: Vec<Market>,
}

#[derive(Clone, Debug, Deserialize)]
struct Market {
    ticker: String,
    #[serde(default)]
    kind: i32,
    #[serde(default)]
    tick_size: i64,
    #[serde(default)]
    min_price_ticks: i64,
    #[serde(default)]
    max_price_ticks: i64,
}

#[derive(Clone, Debug, Deserialize)]
struct LoginResponse {
    token: String,
}

#[derive(Clone, Debug)]
struct Bot {
    user_id: String,
    token: String,
}

#[derive(Debug, Serialize)]
struct DepositRequest {
    amount_usdc: i64,
    note: String,
}

#[derive(Debug, Serialize)]
struct OrderRequest {
    client_order_id: String,
    ticker: String,
    side: &'static str,
    action: &'static str,
    price_ticks: i64,
    count: i64,
    tif: &'static str,
    post_only: bool,
    reduce_only: bool,
    stp: &'static str,
}

#[derive(Debug, Default)]
struct OrderOutcome {
    accepted: bool,
    fill_count: usize,
    order_id: Option<String>,
}

#[derive(Clone, Debug)]
struct LiveOrder {
    bot: Bot,
    order_id: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let config = Config::from_env()?;
    sarvex_runtime::init_tracing("trade-bots");
    tracing::info!(
        bot_count = config.bot_count,
        funding_usdc = config.funding_usdc,
        book_levels = config.book_levels,
        rounds = config.rounds,
        interval_ms = config.interval.as_millis(),
        "trade bot service starting"
    );

    let client = Client::builder()
        .connect_timeout(Duration::from_secs(3))
        .timeout(Duration::from_secs(8))
        .build()
        .context("failed to build trade-bot HTTP client")?;
    let worker_config = config.clone();
    tokio::spawn(async move {
        if let Err(error) = run_worker(client, worker_config).await {
            tracing::error!(error = %error, "trade bot worker stopped");
        }
    });

    sarvex_runtime::run_health_service("trade-bots").await
}

impl Config {
    fn from_env() -> Result<Self> {
        let bot_count = env::var("BOT_COUNT")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(30)
            .clamp(MIN_BOTS, MAX_BOTS);
        let funding_usdc = env::var("BOT_FUND_USDC")
            .ok()
            .and_then(|value| value.parse::<i64>().ok())
            .unwrap_or(100_000)
            .max(1);
        let interval_ms = env::var("BOT_INTERVAL_MS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(1_000)
            .max(100);
        let rounds = env::var("BOT_ROUNDS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(0);
        let book_levels = env::var("BOT_BOOK_LEVELS")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(2)
            .clamp(1, 8);
        let rest_url = env::var("GW_REST_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:18080".to_owned())
            .trim_end_matches('/')
            .to_owned();
        let tickers = env::var("BOT_TICKERS")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|ticker| !ticker.is_empty())
            .map(str::to_owned)
            .collect();
        Ok(Self {
            rest_url,
            bot_count,
            funding_usdc,
            book_levels,
            interval: Duration::from_millis(interval_ms),
            rounds,
            tickers,
        })
    }
}

async fn run_worker(client: Client, config: Config) -> Result<()> {
    let bots = loop {
        match provision_bots(&client, &config).await {
            Ok(bots) => break bots,
            Err(error) => {
                tracing::warn!(error = %error, "gateway not ready for bot provisioning; retrying");
                tokio::time::sleep(Duration::from_secs(3)).await;
            }
        }
    };
    let mut round = 0_u64;
    let mut markets = Vec::new();
    let mut live_quotes = Vec::new();

    loop {
        if markets.is_empty() {
            match load_markets(&client, &config).await {
                Ok(value) => markets = value,
                Err(error) => {
                    tracing::warn!(error = %error, "market catalog unavailable; retrying");
                    tokio::time::sleep(Duration::from_secs(3)).await;
                    continue;
                }
            }
            if markets.is_empty() {
                tracing::warn!("no open markets available; retrying");
                tokio::time::sleep(Duration::from_secs(3)).await;
                continue;
            }
            tracing::info!(
                markets = markets.len(),
                "trade bots discovered open markets"
            );
        }

        round = round.saturating_add(1);
        cancel_live_quotes(&client, &config.rest_url, &mut live_quotes).await;
        live_quotes = seed_books(
            &client,
            &config.rest_url,
            &bots,
            &markets,
            round,
            config.book_levels,
        )
        .await;
        let mut submissions = 0_usize;
        let mut fills = 0_usize;
        for (market_index, market) in markets.iter().enumerate() {
            let (submitted, matched) = run_market_round(
                &client,
                &config.rest_url,
                &bots,
                market,
                round,
                market_index,
            )
            .await;
            submissions += submitted;
            fills += matched;
        }
        tracing::info!(
            round,
            submissions,
            fills,
            markets = markets.len(),
            "trade bot round complete"
        );

        if config.rounds > 0 && round >= config.rounds {
            break;
        }
        tokio::time::sleep(config.interval).await;

        // A contract can close while the bot process remains alive. Refreshing
        // the catalog keeps the simulator from repeatedly submitting stale orders.
        if round.is_multiple_of(30) {
            match load_markets(&client, &config).await {
                Ok(value) => markets = value,
                Err(error) => tracing::warn!(error = %error, "market catalog refresh failed"),
            }
        }
    }
    Ok(())
}

async fn provision_bots(client: &Client, config: &Config) -> Result<Vec<Bot>> {
    let mut bots = Vec::with_capacity(config.bot_count);
    for index in 1..=config.bot_count {
        let user_id = format!("u_bot_{index:03}");
        let token: LoginResponse = post_json(
            client,
            &format!("{}/v1/auth/login", config.rest_url),
            None,
            None,
            serde_json::json!({"user_id": user_id}),
        )
        .await
        .with_context(|| format!("login failed for {user_id}"))?;
        if token.token.is_empty() {
            anyhow::bail!("login returned an empty token for {user_id}");
        }
        let funding_key = format!("trade-bot-funding-v1:{user_id}");
        let _: serde_json::Value = post_json(
            client,
            &format!("{}/v1/demo/deposits/credit", config.rest_url),
            Some(&token.token),
            Some(&funding_key),
            serde_json::json!(DepositRequest {
                amount_usdc: config.funding_usdc,
                note: "Sarvex deterministic trade bot funding".to_owned(),
            }),
        )
        .await
        .with_context(|| format!("funding failed for {user_id}"))?;
        bots.push(Bot {
            user_id,
            token: token.token,
        });
    }
    tracing::info!(bots = bots.len(), "trade bots provisioned and funded");
    Ok(bots)
}

async fn load_markets(client: &Client, config: &Config) -> Result<Vec<Market>> {
    let response: MarketList = client
        .get(format!(
            "{}/v1/markets?state=OPEN&limit=500",
            config.rest_url
        ))
        .send()
        .await
        .context("market listing request failed")?
        .error_for_status()
        .context("market listing returned an error")?
        .json()
        .await
        .context("invalid market listing response")?;
    let mut markets: Vec<Market> = response
        .contracts
        .into_iter()
        .filter(|market| {
            !market.ticker.is_empty() && market.max_price_ticks > market.min_price_ticks
        })
        .filter(|market| {
            config.tickers.is_empty()
                || config.tickers.iter().any(|ticker| ticker == &market.ticker)
        })
        .collect();
    markets.sort_by(|left, right| left.ticker.cmp(&right.ticker));
    Ok(markets)
}

async fn seed_books(
    client: &Client,
    rest_url: &str,
    bots: &[Bot],
    markets: &[Market],
    round: u64,
    book_levels: usize,
) -> Vec<LiveOrder> {
    let maker_count = bots.len() / 3;
    let levels = maker_count.min(book_levels);
    let mut live_quotes = Vec::new();
    for (market_index, market) in markets.iter().enumerate() {
        let fair = fair_price(market, round, market_index);
        let step = price_step(market);
        for level in 1..=levels {
            let distance = step.saturating_mul(level as i64);
            let bid = align_price(market, fair.saturating_sub(distance));
            let ask = align_price(market, fair.saturating_add(distance));
            let count = order_count(market, round + level as u64);
            let bid_bot = &bots[(level - 1) % maker_count];
            let ask_bot = &bots[maker_count + ((level - 1) % maker_count)];
            let bid_outcome = submit_order(
                client,
                rest_url,
                bid_bot,
                market,
                round,
                level as u64,
                "BUY",
                bid,
                count,
                "GTC",
            )
            .await;
            let ask_outcome = submit_order(
                client,
                rest_url,
                ask_bot,
                market,
                round,
                level as u64,
                "SELL",
                ask,
                count,
                "GTC",
            )
            .await;
            if let Some(order_id) = bid_outcome.order_id {
                live_quotes.push(LiveOrder {
                    bot: bid_bot.clone(),
                    order_id,
                });
            }
            if let Some(order_id) = ask_outcome.order_id {
                live_quotes.push(LiveOrder {
                    bot: ask_bot.clone(),
                    order_id,
                });
            }
        }
    }
    live_quotes
}

async fn cancel_live_quotes(client: &Client, rest_url: &str, live_quotes: &mut Vec<LiveOrder>) {
    let previous = std::mem::take(live_quotes);
    for quote in previous {
        let key = format!("bot-quote-cancel-v1:{}", quote.order_id);
        let result = client
            .post(format!("{rest_url}/v1/orders/{}/cancel", quote.order_id))
            .bearer_auth(&quote.bot.token)
            .header("Idempotency-Key", key)
            .send()
            .await;
        if let Ok(response) = result {
            if !response.status().is_success() {
                tracing::debug!(
                    user = %quote.bot.user_id,
                    order_id = %quote.order_id,
                    status = %response.status(),
                    "quote cancellation was not accepted"
                );
            }
        }
    }
}

async fn run_market_round(
    client: &Client,
    rest_url: &str,
    bots: &[Bot],
    market: &Market,
    round: u64,
    market_index: usize,
) -> (usize, usize) {
    let maker_count = bots.len() / 3;
    let taker_start = maker_count * 2;
    let taker_count = bots.len() - taker_start;
    let fair = fair_price(market, round, market_index);
    let step = price_step(market);
    let buy_price = align_price(market, fair.saturating_add(step.saturating_mul(10)));
    let sell_price = align_price(market, fair.saturating_sub(step.saturating_mul(10)));
    let count = order_count(market, round);
    let buy_bot = &bots[taker_start + ((round as usize + market_index) % taker_count)];
    let sell_bot = &bots[taker_start + ((round as usize + market_index + 1) % taker_count)];
    let buy = submit_order(
        client, rest_url, buy_bot, market, round, 10_001, "BUY", buy_price, count, "IOC",
    )
    .await;
    let sell = submit_order(
        client, rest_url, sell_bot, market, round, 10_002, "SELL", sell_price, count, "IOC",
    )
    .await;
    (
        usize::from(buy.accepted) + usize::from(sell.accepted),
        buy.fill_count + sell.fill_count,
    )
}

#[allow(clippy::too_many_arguments)]
async fn submit_order(
    client: &Client,
    rest_url: &str,
    bot: &Bot,
    market: &Market,
    round: u64,
    sequence: u64,
    action: &'static str,
    price_ticks: i64,
    count: i64,
    tif: &'static str,
) -> OrderOutcome {
    let client_order_id = format!(
        "bot-{}-{}-{}-{}",
        bot.user_id, market.ticker, round, sequence
    );
    let request = OrderRequest {
        client_order_id: client_order_id.clone(),
        ticker: market.ticker.clone(),
        side: if market.kind == 2 { "LONG" } else { "YES" },
        action,
        price_ticks,
        count,
        tif,
        post_only: tif == "GTC",
        reduce_only: false,
        stp: "TAKER_AT_CROSS",
    };
    match post_json::<serde_json::Value>(
        client,
        &format!("{rest_url}/v1/orders"),
        Some(&bot.token),
        Some(&client_order_id),
        serde_json::to_value(request).expect("order request serializes"),
    )
    .await
    {
        Ok(response) => {
            let reject = response
                .get("reject_code")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            if !reject.is_empty() {
                tracing::debug!(user = %bot.user_id, ticker = %market.ticker, reject, "bot order rejected");
                OrderOutcome::default()
            } else {
                OrderOutcome {
                    accepted: true,
                    fill_count: response
                        .get("fills")
                        .and_then(|value| value.as_array())
                        .map_or(0, Vec::len),
                    order_id: response
                        .get("order")
                        .and_then(|value| value.get("order_id"))
                        .and_then(|value| value.as_str())
                        .map(str::to_owned),
                }
            }
        }
        Err(error) => {
            tracing::debug!(user = %bot.user_id, ticker = %market.ticker, error = %error, "bot order request failed");
            OrderOutcome::default()
        }
    }
}

async fn post_json<T: for<'de> Deserialize<'de>>(
    client: &Client,
    url: &str,
    token: Option<&str>,
    idempotency_key: Option<&str>,
    body: serde_json::Value,
) -> Result<T> {
    let mut request = client.post(url).json(&body);
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    if let Some(key) = idempotency_key {
        request = request.header("Idempotency-Key", key);
    }
    let response = request.send().await.context("HTTP request failed")?;
    let status = response.status();
    let body = response.text().await.context("HTTP response read failed")?;
    if status != StatusCode::OK && !status.is_success() {
        anyhow::bail!("HTTP {}: {}", status, body);
    }
    serde_json::from_str(&body).with_context(|| format!("invalid JSON response from {url}"))
}

fn price_step(market: &Market) -> i64 {
    market.tick_size.max(1)
}

fn order_count(market: &Market, value: u64) -> i64 {
    if market.kind == 2 {
        1
    } else {
        3 + (value % 4) as i64
    }
}

fn fair_price(market: &Market, round: u64, market_index: usize) -> i64 {
    let min = market.min_price_ticks;
    let max = market.max_price_ticks;
    let width = max.saturating_sub(min);
    let center = min.saturating_add(width / 2);
    let hash = market.ticker.bytes().fold(0_u64, |state, byte| {
        state.wrapping_mul(31).wrapping_add(byte as u64)
    });
    let wave = ((round
        .wrapping_mul(37)
        .wrapping_add(hash)
        .wrapping_add(market_index as u64 * 17))
        % 161) as i64
        - 80;
    let swing = ((width as i128 * wave as i128) / 1000) as i64;
    align_price(market, center.saturating_add(swing))
}

fn align_price(market: &Market, value: i64) -> i64 {
    let min = market.min_price_ticks;
    let max = market.max_price_ticks;
    let tick = market.tick_size.max(1);
    let bounded = value.clamp(min, max);
    let offset = (bounded.saturating_sub(min) / tick).saturating_mul(tick);
    min.saturating_add(offset).min(max)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn market() -> Market {
        Market {
            ticker: "TEST".to_owned(),
            kind: 1,
            tick_size: 2,
            min_price_ticks: 1,
            max_price_ticks: 99,
        }
    }

    #[test]
    fn prices_stay_inside_contract_bounds_and_on_tick() {
        let market = market();
        for round in 0..100 {
            let price = fair_price(&market, round, 0);
            assert!((1..=99).contains(&price));
            assert_eq!((price - 1) % 2, 0);
        }
    }

    #[test]
    fn bot_count_is_clamped_to_demo_range() {
        assert_eq!(5_usize.clamp(MIN_BOTS, MAX_BOTS), 20);
        assert_eq!(30_usize.clamp(MIN_BOTS, MAX_BOTS), 30);
        assert_eq!(100_usize.clamp(MIN_BOTS, MAX_BOTS), 50);
    }
}
