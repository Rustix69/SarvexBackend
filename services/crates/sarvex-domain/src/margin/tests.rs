use super::*;

const USDC: i64 = 1_000_000;
const NUMERIC: OutcomeKind = OutcomeKind::Numeric;

/// CPI future: band 4.00%..6.00% in bp, 10 USDC per bp; binary "CPI > 4.50%".
fn cpi_book() -> EventBook {
    EventBook::new(
        NUMERIC,
        vec![
            Instrument::banded("SXF-CPI", 400, 600, 10 * USDC),
            Instrument::binary_range("SX-CPI-T450", Some(451), None),
        ],
        None,
    )
    .unwrap()
}

fn pos(instrument: usize, qty: i64) -> Position {
    Position { instrument, qty }
}

fn order(instrument: usize, qty: i64, price_ticks: i64) -> WorkingOrder {
    WorkingOrder {
        instrument,
        qty,
        price_ticks,
    }
}

// ---------------------------------------------------------------- vectors

#[test]
fn single_positions_reproduce_model_a_locks() {
    let book = cpi_book();
    // Long future at 5.00%: pays (500 - 400) * 10 = 1,000 USDC premium, owes nothing more.
    assert_eq!(book.trade_value_micro(0, 500, 1).unwrap(), 1_000 * USDC);
    assert_eq!(book.requirement(&[pos(0, 1)]).unwrap(), 0);
    // Short future: receives 1,000, locks the full band 2,000: net (600 - 500) * 10 = 1,000.
    assert_eq!(book.trade_value_micro(0, 500, -1).unwrap(), -1_000 * USDC);
    let short = book.event_margin(&[pos(0, -1)], &[]).unwrap();
    assert_eq!(short.position_requirement_micro, 2_000 * USDC);
    assert_eq!(short.worst_outcome, Some(Outcome::Numeric(600)));
    // NO holder (short YES at 30c): receives 0.30, locks 1.00: net 0.70 = the NO price.
    assert_eq!(book.trade_value_micro(1, 30, -1).unwrap(), -300_000);
    assert_eq!(book.requirement(&[pos(1, -1)]).unwrap(), USDC);
}

#[test]
fn handover_ttf_vectors() {
    // TTF: divider 100, tv = 0.10 USDC, band 2000..8000, entry 4000.
    let book = EventBook::new(
        NUMERIC,
        vec![Instrument::banded("SXF-TTF", 2_000, 8_000, 100_000)],
        None,
    )
    .unwrap();
    let long_premium = book.trade_value_micro(0, 4_000, 1).unwrap();
    let short_premium = book.trade_value_micro(0, 4_000, -1).unwrap();
    let short_lock = book.requirement(&[pos(0, -1)]).unwrap();
    assert_eq!(long_premium, 200 * USDC); // long lock
    assert_eq!(short_lock + short_premium, 400 * USDC); // short lock
    for (final_ticks, long_payout, long_net) in
        [(4_500, 250, 50), (9_500, 600, 400), (1_500, 0, -200)]
    {
        let x = Outcome::Numeric(final_ticks);
        let amounts = book
            .settle_accounts(&[vec![pos(0, 1)], vec![pos(0, -1)]], &x)
            .unwrap();
        assert_eq!(amounts[0], long_payout * USDC);
        assert_eq!(amounts[1], -long_payout * USDC);
        assert_eq!(amounts[0] - long_premium, long_net * USDC);
        assert_eq!(amounts[1] - short_premium, -long_net * USDC);
    }
}

#[test]
fn cpi_hedge_cuts_capital_from_1100_to_600() {
    let book = cpi_book();
    // Long 1 future at 5.00% and buy 500 NO on "CPI > 4.50%" at $0.20 (= sell 500 YES at $0.80).
    let positions = [pos(0, 1), pos(1, -500)];
    let premium =
        book.trade_value_micro(0, 500, 1).unwrap() + book.trade_value_micro(1, 80, -500).unwrap();
    let margin = book.event_margin(&positions, &[]).unwrap();
    assert_eq!(premium, 600 * USDC);
    assert_eq!(margin.position_requirement_micro, 0);
    assert_eq!(margin.worst_outcome, None);
    assert_eq!(premium + margin.margin_micro, 600 * USDC); // cross margin
    assert_eq!(premium + margin.standalone_micro, 1_100 * USDC); // Model A today
    assert_eq!(margin.savings_micro(), 500 * USDC);
    // Worst case is a 4.00% print: settlement 0 against 600 paid.
    assert_eq!(
        book.settlement_micro(&positions, &Outcome::Numeric(400))
            .unwrap(),
        0
    );
    assert_eq!(
        book.settlement_micro(&positions, &Outcome::Numeric(451))
            .unwrap(),
        10 * USDC
    );
    assert_eq!(
        book.settlement_micro(&positions, &Outcome::Numeric(450))
            .unwrap(),
        500 * USDC
    );
}

#[test]
fn working_orders_count_once_for_any_fill_combination() {
    let book = cpi_book();
    // The same two legs as resting orders on a flat account.
    let orders = [order(0, 1, 500), order(1, -500, 80)];
    let margin = book.event_margin(&[], &orders).unwrap();
    // The future bid alone could cost 1,000 if only it fills; the hedge adds nothing.
    assert_eq!(margin.margin_micro, 1_000 * USDC);
    assert_eq!(margin.position_requirement_micro, 0);
    assert_eq!(margin.standalone_micro, 1_100 * USDC);
    // A two-sided quote on one contract: bid 45 and ask 55 never both count in full.
    let quote = book
        .event_margin(&[], &[order(1, 100, 45), order(1, -100, 55)])
        .unwrap();
    assert_eq!(quote.margin_micro, 45 * USDC);
    assert_eq!(quote.standalone_micro, 90 * USDC);
}

#[test]
fn categorical_buckets_only_one_can_win() {
    let book = EventBook::new(
        OutcomeKind::Categorical,
        vec![
            Instrument::binary_category("FED-CUT", &["CUT"]),
            Instrument::binary_category("FED-HOLD", &["HOLD"]),
            Instrument::binary_category("FED-H25", &["HIKE_25"]),
            Instrument::binary_category("FED-HGT25", &["HIKE_GT25"]),
        ],
        None,
    )
    .unwrap();
    // Short YES 100 on CUT and on HOLD: at most one pays, so 100 USDC, not 200.
    let short_two = book
        .event_margin(&[pos(0, -100), pos(1, -100)], &[])
        .unwrap();
    assert_eq!(short_two.position_requirement_micro, 100 * USDC);
    assert_eq!(short_two.standalone_micro, 200 * USDC);
    // Long the whole strip: always paid 1 USDC, never owes.
    let strip: Vec<Position> = (0..4).map(|i| pos(i, 1)).collect();
    assert_eq!(book.requirement(&strip).unwrap(), 0);
    assert_eq!(
        book.settlement_micro(&strip, &Outcome::OtherCategory)
            .unwrap(),
        0
    );
    assert_eq!(
        book.settlement_micro(&strip, &Outcome::Category("HOLD".into()))
            .unwrap(),
        USDC
    );
}

#[test]
fn validation_refuses_bad_input() {
    assert!(matches!(
        EventBook::new(
            OutcomeKind::Categorical,
            vec![Instrument::banded("F", 0, 10, 1)],
            None
        ),
        Err(MarginError::OutcomeKindMismatch { .. })
    ));
    assert!(matches!(
        EventBook::new(
            NUMERIC,
            vec![Instrument::binary_range("B", Some(10), Some(5))],
            None
        ),
        Err(MarginError::InvalidInstrument { .. })
    ));
    assert!(matches!(
        EventBook::new(NUMERIC, vec![Instrument::banded("F", 10, 10, 1)], None),
        Err(MarginError::InvalidInstrument { .. })
    ));
    let book = cpi_book();
    assert!(matches!(
        book.event_margin(&[], &[order(1, 1, 100)]),
        Err(MarginError::InvalidPrice { .. })
    ));
    assert_eq!(
        book.event_margin(&[], &[order(0, 0, 500)]),
        Err(MarginError::ZeroQuantity)
    );
    assert_eq!(
        book.requirement(&[pos(9, 1)]),
        Err(MarginError::UnknownInstrument(9))
    );
    assert!(matches!(
        book.settlement_micro(&[pos(0, 1)], &Outcome::OtherCategory),
        Err(MarginError::InvalidOutcome(_))
    ));
    assert_eq!(
        book.requirement(&[pos(0, -i64::MAX)]),
        Err(MarginError::Overflow)
    );
    assert!(matches!(
        book.settle_accounts(&[vec![pos(0, 1)]], &Outcome::Numeric(500)),
        Err(MarginError::NotZeroSum(_))
    ));
}

// ---------------------------------------------------------------- properties

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn range(&mut self, lo: i64, hi: i64) -> i64 {
        lo + (self.next() % ((hi - lo + 1) as u64)) as i64
    }
    fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }
}

const X_MIN: i64 = -5;
const X_MAX: i64 = 125;
const CATEGORIES: [&str; 4] = ["A", "B", "C", "D"];

fn random_book(rng: &mut Rng) -> EventBook {
    if rng.chance(25) {
        let count = rng.range(1, 4);
        let instruments = (0..count)
            .map(|i| {
                let yes: Vec<&str> = CATEGORIES
                    .iter()
                    .copied()
                    .filter(|_| rng.chance(40))
                    .collect();
                let yes = if yes.is_empty() { vec!["A"] } else { yes };
                Instrument::binary_category(format!("C{i}"), &yes)
            })
            .collect();
        return EventBook::new(OutcomeKind::Categorical, instruments, None).unwrap();
    }
    let mut instruments = Vec::new();
    if rng.chance(80) {
        let lower = rng.range(0, 40);
        let upper = lower + rng.range(1, 80);
        let tick_value = [1, 3, 1_000, 25_000][rng.range(0, 3) as usize];
        instruments.push(Instrument::banded("F", lower, upper, tick_value));
    }
    for i in 0..rng.range(1, 4) {
        let lo = rng.chance(75).then(|| rng.range(0, 120));
        let hi = rng.chance(50).then(|| rng.range(lo.unwrap_or(0), 120));
        instruments.push(Instrument::binary_range(format!("B{i}"), lo, hi));
    }
    EventBook::new(NUMERIC, instruments, None).unwrap()
}

fn all_outcomes(book: &EventBook) -> Vec<Outcome> {
    match book.kind() {
        OutcomeKind::Numeric => match book.grid() {
            // Every grid point in a range that reaches past every random kink.
            Some(grid) => (X_MIN - 40..=X_MAX + 40)
                .filter(|x| grid.contains(*x))
                .map(Outcome::Numeric)
                .collect(),
            None => (X_MIN..=X_MAX).map(Outcome::Numeric).collect(),
        },
        OutcomeKind::Categorical => {
            let mut out: Vec<Outcome> = CATEGORIES
                .iter()
                .map(|c| Outcome::Category((*c).into()))
                .collect();
            out.push(Outcome::OtherCategory);
            out.push(Outcome::Category("UNLISTED".into()));
            out
        }
    }
}

fn random_price(rng: &mut Rng, book: &EventBook, instrument: usize) -> i64 {
    let item = &book.instruments()[instrument];
    rng.range(item.min_price_ticks, item.max_price_ticks)
}

fn random_positions(rng: &mut Rng, book: &EventBook) -> Vec<Position> {
    let mut out = Vec::new();
    for i in 0..book.instruments().len() {
        if rng.chance(70) {
            out.push(pos(i, rng.range(-20, 20)));
        }
    }
    out
}

fn random_orders(rng: &mut Rng, book: &EventBook, max: i64) -> Vec<WorkingOrder> {
    (0..rng.range(0, max))
        .map(|_| {
            let instrument = rng.range(0, book.instruments().len() as i64 - 1) as usize;
            let mut qty = rng.range(-10, 10);
            if qty == 0 {
                qty = 1;
            }
            order(instrument, qty, random_price(rng, book, instrument))
        })
        .collect()
}

/// Independent brute force: every outcome, every subset of order fills.
fn brute_force_margin(book: &EventBook, positions: &[Position], orders: &[WorkingOrder]) -> i128 {
    let outcomes = all_outcomes(book);
    let mut best = i128::MIN;
    for mask in 0_u32..(1 << orders.len()) {
        let mut held: Vec<Position> = positions.to_vec();
        let mut premium = 0_i128;
        for (bit, o) in orders.iter().enumerate() {
            if mask & (1 << bit) != 0 {
                held.push(pos(o.instrument, o.qty));
                premium += i128::from(
                    book.trade_value_micro(o.instrument, o.price_ticks, o.qty)
                        .unwrap(),
                );
            }
        }
        let owed = outcomes
            .iter()
            .map(|x| -i128::from(book.settlement_micro(&held, x).unwrap()))
            .max()
            .unwrap()
            .max(0);
        best = best.max(owed + premium);
    }
    best
}

#[test]
fn margin_equals_brute_force_worst_case() {
    let mut rng = Rng(0x5eed_1234_abcd_0001);
    for _ in 0..1_500 {
        let book = random_book(&mut rng);
        let positions = random_positions(&mut rng, &book);
        let orders = random_orders(&mut rng, &book, 5);
        let margin = book.event_margin(&positions, &orders).unwrap();
        assert_eq!(
            i128::from(margin.margin_micro),
            brute_force_margin(&book, &positions, &orders),
            "book {book:?} positions {positions:?} orders {orders:?}"
        );
        assert_eq!(
            i128::from(margin.position_requirement_micro),
            brute_force_margin(&book, &positions, &[])
        );
        assert_eq!(
            book.worst_position_loss_micro(&positions).unwrap(),
            margin.position_requirement_micro
        );
        assert!(margin.position_requirement_micro <= margin.margin_micro);
        assert!(
            margin.margin_micro <= margin.standalone_micro,
            "cross margin above standalone"
        );
        if let Some(worst) = &margin.worst_outcome {
            assert_eq!(
                -i128::from(book.settlement_micro(&positions, worst).unwrap()),
                i128::from(margin.position_requirement_micro)
            );
        }
    }
}

#[test]
fn fills_never_need_new_cash() {
    let mut rng = Rng(0x5eed_1234_abcd_0002);
    for _ in 0..3_000 {
        let book = random_book(&mut rng);
        let mut positions = random_positions(&mut rng, &book);
        let mut orders = random_orders(&mut rng, &book, 6);
        while let Some(index) =
            (!orders.is_empty()).then(|| rng.range(0, orders.len() as i64 - 1) as usize)
        {
            let before = book.event_margin(&positions, &orders).unwrap().margin_micro;
            let o = orders[index];
            let filled = o.qty.signum() * rng.range(1, o.qty.abs());
            let item = &book.instruments()[o.instrument];
            // A fill is at the limit or better: buyers pay less, sellers receive more.
            let price = if o.qty > 0 {
                rng.range(item.min_price_ticks, o.price_ticks)
            } else {
                rng.range(o.price_ticks, item.max_price_ticks)
            };
            let premium = book.trade_value_micro(o.instrument, price, filled).unwrap();
            positions.push(pos(o.instrument, filled));
            if filled == o.qty {
                orders.remove(index);
            } else {
                orders[index].qty -= filled;
            }
            let after = book.event_margin(&positions, &orders).unwrap().margin_micro;
            assert!(
                after + premium <= before,
                "fill needed cash: before {before} after {after} premium {premium}"
            );
        }
    }
}

// ---------------------------------------------------------------- reference clearing flow

/// In-memory model of the ledger accounts each user has per event.
struct Clearing {
    book: EventBook,
    cash: Vec<i64>,
    margin: Vec<i64>,
    clearing: i64,
    positions: Vec<Vec<Position>>,
    orders: Vec<(usize, WorkingOrder)>,
    total: i128,
    fills: usize,
    rejected: usize,
}

impl Clearing {
    fn new(book: EventBook, cash: Vec<i64>) -> Self {
        let users = cash.len();
        let total = cash.iter().map(|c| i128::from(*c)).sum();
        Self {
            book,
            cash,
            margin: vec![0; users],
            clearing: 0,
            positions: vec![Vec::new(); users],
            orders: Vec::new(),
            total,
            fills: 0,
            rejected: 0,
        }
    }

    fn required(&self, user: usize) -> i64 {
        let orders: Vec<WorkingOrder> = self
            .orders
            .iter()
            .filter(|(u, _)| *u == user)
            .map(|(_, o)| *o)
            .collect();
        self.book
            .event_margin(&self.positions[user], &orders)
            .unwrap()
            .margin_micro
    }

    /// risk-svc ReserveOrder: the only place that can refuse for funds.
    fn place(&mut self, user: usize, o: WorkingOrder) -> bool {
        self.orders.push((user, o));
        let delta = self.required(user) - self.margin[user];
        if delta > self.cash[user] {
            self.orders.pop();
            self.rejected += 1;
            return false;
        }
        self.cash[user] -= delta;
        self.margin[user] += delta;
        true
    }

    /// risk-svc ReleaseOrder.
    fn cancel(&mut self, index: usize) {
        let (user, _) = self.orders.remove(index);
        let release = self.margin[user] - self.required(user);
        assert!(release >= 0, "a cancel raised margin");
        self.margin[user] -= release;
        self.cash[user] += release;
    }

    /// risk-svc ApplyFill: premium from buyer margin via clearing to seller cash, then re-margin.
    fn fill(&mut self, buy: usize, sell: usize, qty: i64, price: i64) {
        let (buyer, bo) = self.orders[buy];
        let (seller, _) = self.orders[sell];
        let premium = self
            .book
            .trade_value_micro(bo.instrument, price, qty)
            .unwrap();
        self.fills += 1;
        assert!(
            self.margin[buyer] >= premium,
            "buyer margin cannot fund the premium"
        );
        self.margin[buyer] -= premium;
        self.clearing += premium;
        self.clearing -= premium;
        self.cash[seller] += premium;
        self.positions[buyer].push(pos(bo.instrument, qty));
        self.positions[seller].push(pos(bo.instrument, -qty));
        self.orders[buy].1.qty -= qty;
        self.orders[sell].1.qty += qty;
        self.orders.retain(|(_, o)| o.qty != 0);
        for user in [buyer, seller] {
            let delta = self.required(user) - self.margin[user];
            assert!(
                delta <= self.cash[user],
                "fill left user {user} short by {}",
                delta - self.cash[user]
            );
            self.cash[user] -= delta;
            self.margin[user] += delta;
        }
    }

    /// settlement-svc SettleEvent: debit losers' margin, credit winners' cash, release the rest.
    fn settle(&mut self, outcome: &Outcome) {
        while !self.orders.is_empty() {
            self.cancel(0);
        }
        let amounts = self.book.settle_accounts(&self.positions, outcome).unwrap();
        for (user, amount) in amounts.iter().enumerate() {
            if *amount < 0 {
                assert!(
                    self.margin[user] >= -amount,
                    "margin does not cover the settlement debit"
                );
                self.margin[user] += amount;
                self.clearing -= amount;
            }
        }
        for (user, amount) in amounts.iter().enumerate() {
            if *amount > 0 {
                self.clearing -= amount;
                self.cash[user] += amount;
            }
            self.cash[user] += self.margin[user];
            self.margin[user] = 0;
            self.positions[user].clear();
        }
    }

    fn check(&self) {
        assert_eq!(self.clearing, 0, "clearing account must net to zero");
        assert!(self.cash.iter().all(|c| *c >= 0), "negative cash");
        assert!(self.margin.iter().all(|m| *m >= 0), "negative margin");
        for user in 0..self.cash.len() {
            assert_eq!(
                self.margin[user],
                self.required(user),
                "posted margin drifted for user {user}"
            );
        }
        let total: i128 = self
            .cash
            .iter()
            .chain(self.margin.iter())
            .map(|v| i128::from(*v))
            .sum();
        assert_eq!(total, self.total, "money was created or destroyed");
    }
}

#[test]
fn reference_flow_cpi_hedge() {
    let mut flow = Clearing::new(cpi_book(), vec![1_000 * USDC, 100_000 * USDC]);
    // Trader 0 bids the future at 500 and offers 500 YES at 80; the maker takes both.
    assert!(flow.place(0, order(0, 1, 500)));
    assert!(flow.place(0, order(1, -500, 80)));
    assert_eq!(flow.margin[0], 1_000 * USDC);
    assert!(flow.place(1, order(0, -1, 500)));
    assert!(flow.place(1, order(1, 500, 80)));
    flow.fill(0, 2, 1, 500); // trader buys the future from the maker
    flow.check();
    // Filled orders are removed, so the book is now [trader YES offer, maker YES bid].
    flow.fill(1, 0, 500, 80); // maker buys 500 YES from the trader
    flow.check();
    assert_eq!(flow.cash[0], 400 * USDC);
    assert_eq!(flow.margin[0], 0);
    flow.settle(&Outcome::Numeric(451));
    flow.check();
    assert_eq!(flow.cash[0], 410 * USDC); // -590 USDC: future -490, NO -100
}

#[test]
fn reference_flow_random_markets() {
    let mut rng = Rng(0x5eed_1234_abcd_0003);
    let (mut fills, mut rejected, mut hedged) = (0, 0, 0);
    for _ in 0..400 {
        let book = random_book(&mut rng);
        let users = 4;
        let cash: Vec<i64> = (0..users).map(|_| rng.range(0, 400) * USDC).collect();
        let mut flow = Clearing::new(book.clone(), cash);
        for _ in 0..60 {
            match rng.range(0, 9) {
                0..=4 => {
                    let user = rng.range(0, users as i64 - 1) as usize;
                    let instrument = rng.range(0, book.instruments().len() as i64 - 1) as usize;
                    let mut qty = rng.range(-30, 30);
                    if qty == 0 {
                        qty = -1;
                    }
                    let price = random_price(&mut rng, &book, instrument);
                    flow.place(user, order(instrument, qty, price));
                }
                5 if !flow.orders.is_empty() => {
                    let index = rng.range(0, flow.orders.len() as i64 - 1) as usize;
                    flow.cancel(index);
                }
                _ => {
                    let crossing: Vec<(usize, usize)> = (0..flow.orders.len())
                        .flat_map(|b| (0..flow.orders.len()).map(move |s| (b, s)))
                        .filter(|(b, s)| {
                            let (bu, bo) = flow.orders[*b];
                            let (su, so) = flow.orders[*s];
                            bu != su
                                && bo.instrument == so.instrument
                                && bo.qty > 0
                                && so.qty < 0
                                && bo.price_ticks >= so.price_ticks
                        })
                        .collect();
                    if let Some(&(b, s)) =
                        crossing.get(rng.range(0, crossing.len().max(1) as i64 - 1) as usize)
                    {
                        let qty = rng.range(1, flow.orders[b].1.qty.min(-flow.orders[s].1.qty));
                        let price =
                            rng.range(flow.orders[s].1.price_ticks, flow.orders[b].1.price_ticks);
                        flow.fill(b, s, qty, price);
                    }
                }
            }
            flow.check();
        }
        let outcomes = all_outcomes(&book);
        let outcome = outcomes[rng.range(0, outcomes.len() as i64 - 1) as usize].clone();
        for user in 0..users {
            let margin = book.event_margin(&flow.positions[user], &[]).unwrap();
            if margin.savings_micro() > 0 {
                hedged += 1;
            }
        }
        fills += flow.fills;
        rejected += flow.rejected;
        flow.settle(&outcome);
        flow.check();
        assert!(flow.margin.iter().all(|m| *m == 0));
    }
    // The scenario must actually trade, hit the cash limit and net offsetting positions.
    assert!(fills > 2_000, "only {fills} fills");
    assert!(rejected > 100, "only {rejected} rejections");
    assert!(hedged > 100, "only {hedged} accounts with netting");
}

// ---------------------------------------------------------------- CM-09: closing orders

/// Independent model of today's separate holds, one contract at a time:
/// short positions lock their max value; each unit of a futures order that
/// reduces the position (capacity |net|, time priority) locks nothing; every
/// other unit locks its premium (buy) or max value minus premium (sell).
/// Binary holds depend on the action only (risk-svc::required_hold).
fn model_a_standalone(book: &EventBook, positions: &[Position], orders: &[WorkingOrder]) -> i128 {
    let mut net = vec![0_i64; book.instruments().len()];
    for p in positions {
        net[p.instrument] += p.qty;
    }
    let mut total = 0_i128;
    for (i, q) in net.iter().enumerate() {
        if *q < 0 {
            total += book.instruments()[i].payoff.max_value_micro() * i128::from(-q);
        }
    }
    let mut capacity: Vec<i64> = net.iter().map(|q| q.abs()).collect();
    for o in orders {
        let payoff = &book.instruments()[o.instrument].payoff;
        let futures = matches!(payoff, Payoff::Banded { .. });
        let unit = i128::from(
            book.trade_value_micro(o.instrument, o.price_ticks, 1)
                .unwrap(),
        );
        for _ in 0..o.qty.abs() {
            let reduces =
                futures && capacity[o.instrument] > 0 && (net[o.instrument] > 0) != (o.qty > 0);
            if reduces {
                capacity[o.instrument] -= 1;
            } else if o.qty > 0 {
                total += unit;
            } else {
                total += payoff.max_value_micro() - unit;
            }
        }
    }
    total
}

#[test]
fn closing_orders_are_not_margined_as_new_shorts() {
    let book = cpi_book();
    // Long 1 future; offering it at 5.50% only closes: nothing separate, nothing cross.
    let close = book
        .event_margin(&[pos(0, 1)], &[order(0, -1, 550)])
        .unwrap();
    assert_eq!(close.standalone_micro, 0);
    assert_eq!(close.margin_micro, 0);
    // Offering 3 against a long of 1: 1 closes, 2 open a short at (600 - 550) * 10 each.
    let over = book
        .event_margin(&[pos(0, 1)], &[order(0, -3, 550)])
        .unwrap();
    assert_eq!(over.standalone_micro, 2 * 500 * USDC);
    // Two sell-to-close orders share one long: only the first closes.
    let two = book
        .event_margin(&[pos(0, 1)], &[order(0, -1, 550), order(0, -1, 560)])
        .unwrap();
    assert_eq!(two.standalone_micro, 400 * USDC);
    // Binary holds follow the action only, so a sell-to-close of 10 YES at 60 still locks 0.40 each.
    let binary = book
        .event_margin(&[pos(1, 10)], &[order(1, -10, 60)])
        .unwrap();
    assert_eq!(binary.standalone_micro, 4 * USDC);
    assert_eq!(binary.margin_micro, 0);
}

#[test]
fn standalone_matches_a_simulation_of_todays_holds() {
    let mut rng = Rng(0x5eed_1234_abcd_0009);
    for _ in 0..3_000 {
        let book = random_book(&mut rng);
        let positions = random_positions(&mut rng, &book);
        let orders = random_orders(&mut rng, &book, 6);
        let margin = book.event_margin(&positions, &orders).unwrap();
        assert_eq!(
            i128::from(margin.standalone_micro),
            model_a_standalone(&book, &positions, &orders),
            "book {book:?} positions {positions:?} orders {orders:?}"
        );
        assert!(margin.margin_micro <= margin.standalone_micro);
    }
}

// ---------------------------------------------------------------- CM-11: outcome grid

const CPI_GRID: OutcomeGrid = OutcomeGrid {
    step: 10,
    anchor: 0,
    min: None,
    max: None,
};

/// Short 1 CPI future (400..600, 10 USDC/bp) against 100 YES on "CPI >= k"
/// for k = 410, 420, ..., 600: on the 10 bp grid the strip replicates the future.
fn strip_vs_future(grid: Option<OutcomeGrid>) -> i64 {
    let mut instruments = vec![Instrument::banded("SXF-CPI", 400, 600, 10 * USDC)];
    for k in (410..=600).step_by(10) {
        instruments.push(Instrument::binary_range(
            format!("SX-CPI-{k}"),
            Some(k),
            None,
        ));
    }
    let book = EventBook::new(NUMERIC, instruments, grid).unwrap();
    let mut positions = vec![pos(0, -1)];
    positions.extend((1..book.instruments().len()).map(|i| pos(i, 100)));
    book.requirement(&positions).unwrap()
}

#[test]
fn strip_hedges_the_future_exactly_on_the_publication_grid() {
    assert_eq!(strip_vs_future(None), 90 * USDC); // a 4.09% print is possible
    assert_eq!(strip_vs_future(Some(CPI_GRID)), 0);
}

#[test]
fn grid_rules_are_enforced() {
    let grid_book = |grid: OutcomeGrid| {
        EventBook::new(
            NUMERIC,
            vec![Instrument::banded("SXF-CPI", 400, 600, 10 * USDC)],
            Some(grid),
        )
    };
    let book = grid_book(CPI_GRID).unwrap();
    assert!(book
        .settlement_micro(&[pos(0, 1)], &Outcome::Numeric(450))
        .is_ok());
    assert!(matches!(
        book.settlement_micro(&[pos(0, 1)], &Outcome::Numeric(455)),
        Err(MarginError::InvalidOutcome(_))
    ));
    let bounded = grid_book(OutcomeGrid {
        step: 1,
        anchor: 0,
        min: Some(0),
        max: Some(538),
    })
    .unwrap();
    assert!(matches!(
        bounded.settlement_micro(&[pos(0, 1)], &Outcome::Numeric(539)),
        Err(MarginError::InvalidOutcome(_))
    ));
    for bad in [
        OutcomeGrid {
            step: 0,
            ..CPI_GRID
        },
        OutcomeGrid {
            min: Some(20),
            max: Some(10),
            ..CPI_GRID
        },
        OutcomeGrid {
            min: Some(5),
            ..CPI_GRID
        },
    ] {
        assert!(
            matches!(grid_book(bad), Err(MarginError::InvalidGrid(_))),
            "{bad:?}"
        );
    }
    assert!(matches!(
        EventBook::new(
            OutcomeKind::Categorical,
            vec![Instrument::binary_category("C", &["A"])],
            Some(CPI_GRID)
        ),
        Err(MarginError::InvalidGrid(_))
    ));
}

fn random_grid(rng: &mut Rng) -> OutcomeGrid {
    let step = rng.range(1, 12);
    let anchor = rng.range(-6, 6);
    let on_grid = |x: i64| anchor + (x - anchor).div_euclid(step) * step;
    let min = rng.chance(30).then(|| on_grid(rng.range(-10, 60)));
    let max = rng
        .chance(30)
        .then(|| on_grid(rng.range(min.unwrap_or(-10).max(-10) + step, 140)))
        .filter(|max| min.is_none_or(|min| *max >= min));
    OutcomeGrid {
        step,
        anchor,
        min,
        max,
    }
}

#[test]
fn grid_margin_equals_brute_force_over_the_grid() {
    let mut rng = Rng(0x5eed_1234_abcd_000a);
    let mut checked = 0;
    while checked < 1_500 {
        let base = random_book(&mut rng);
        if base.kind() != OutcomeKind::Numeric {
            continue;
        }
        let book = EventBook::new(
            NUMERIC,
            base.instruments().to_vec(),
            Some(random_grid(&mut rng)),
        )
        .unwrap();
        let positions = random_positions(&mut rng, &book);
        let orders = random_orders(&mut rng, &book, 5);
        let margin = book.event_margin(&positions, &orders).unwrap();
        assert_eq!(
            i128::from(margin.margin_micro),
            brute_force_margin(&book, &positions, &orders),
            "book {book:?} positions {positions:?} orders {orders:?}"
        );
        assert_eq!(
            i128::from(margin.position_requirement_micro),
            brute_force_margin(&book, &positions, &[])
        );
        if let Some(Outcome::Numeric(worst)) = margin.worst_outcome {
            assert!(
                book.grid().unwrap().contains(worst),
                "worst outcome {worst} is off the grid"
            );
        }
        // The grid never asks for more than the integer axis.
        let integer = EventBook::new(NUMERIC, book.instruments().to_vec(), None)
            .unwrap()
            .event_margin(&positions, &orders)
            .unwrap();
        assert!(margin.margin_micro <= integer.margin_micro);
        checked += 1;
    }
}
