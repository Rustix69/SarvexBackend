#!/usr/bin/env python3
"""Proposed cross-margin backfill (prerequisite fix 5): outcome axis, payoff rules, rounding.

Reads nothing from the database. Every row below was read by hand from the live contract
terms (settlement_rule.workbook.payoff_formula, title, futures divider and band) on
12 Oct 2026. Output: proposed_payoff_rules.sql (apply only after review) and
backfill_report.md (what is proposed, what needs a decision, what must stay ISOLATED).

Axis convention: raw outcome ticks = published value x futures `divider`. A strict
"> s" is lo = s*d + 1, "< s" is hi = s*d - 1, ">= s" is lo = s*d. `step` is the
published precision in ticks (CPI to 1 decimal at divider 100 is step 10), so the event
grid only holds outcomes the source can actually publish (CM-11).
"""
import json
from decimal import Decimal as D

# event, binary, strike, comparator, divider d, step, note
AUTO = [
    ("XEV-STD-ECO-02", "SX-FEDUB-26DEC-T4.00",        "4.00",  ">",  100,   25, "target-range upper bound moves in 25 bp steps"),
    ("XEV-STD-ECO-03", "SX-USCPIYOY-26SEP-T3.4",      "3.4",   ">",  100,   10, "published to 1 decimal"),
    ("XEV-STD-ECO-04", "SX-USCPIMOM-26SEP-T0.2",      "0.2",   ">",  100,   10, "published to 1 decimal"),
    ("XEV-REG-EU-02",  "SX-EZHICP-26OCT-T3.0",        "3.0",   ">",  100,   10, "flash HICP to 1 decimal"),
    ("XEV-REG-IND-02", "SX-INCPI-26SEP-T4.50",        "4.50",  ">",  100,    1, "stated 2-decimal precision"),
    ("XEV-STD-FIN-04", "SX-UST10Y-26OCT30-T4.90",     "4.90",  ">",  100,    1, "par yield published to 2 decimals"),
    ("XEV-REG-AS-02",  "SX-USDJPY-26OCT30-T155",      "155.00", ">", 100,    1, "ASSUMES the reference is rounded to 2 decimals"),
    ("XEV-STD-COM-01", "SX-WTI-26OCT30-T100",         "100",   ">",  100,    1, "settlement price to 2 decimals"),
    ("XEV-REG-ME-01",  "SX-BRENT-26OCT30-T100",       "100.00", ">", 100,    1, "settlement price to 2 decimals"),
    ("XEV-STD-ECO-05", "SX-AAAGAS-26OCT19-T4.25",     "4.25",  ">", 1000,    1, "AAA publishes 3 decimals"),
    ("XEV-STD-FIN-05", "SX-EURUSD-26OCT16-T1.1600",   "1.1600", ">", 10000,  1, "ASSUMES the reference is rounded to 4 decimals (pips)"),
    ("XEV-REG-IND-06", "SX-DELAQI-26NOV09-T400",      "400",   ">",    1,    1, "AQI is an integer"),
    ("XEV-REG-ME-04",  "SX-HIGHDXB-26OCT15-T38",      "38.0",  ">=",  10,    1, "daily max to 0.1 C"),
    ("XEV-STD-GEO-01", "SX-HORMUZW-26OCT18-T40",      "40",    ">",    1,    1, "count of transit calls"),
    ("XEV-REG-AS-04",  "SX-LPR1Y-26OCT-CUT",          "3.00",  "<",  100,    1, "1Y LPR to 2 decimals (really 5 bp steps; step 1 is conservative)"),
]

DECISION = {
    "XEV-STD-ECO-01": "Rate change (+0.25 pp) against the futures' level axis: needs the prior upper bound as an input, and the event's 4 outcomes as categories or a level grid.",
    "XEV-REG-AS-01": "Rate change versus the prior BOJ target: needs the prior rate.",
    "XEV-REG-EU-01": "Rate change versus the prior ECB DFR: needs the prior rate.",
    "XEV-REG-IND-01": "Rate change versus the prior RBI repo: needs the prior rate.",
    "XEV-REG-ME-02": "Rate change versus the prior CBUAE base rate: needs the prior rate.",
    "XEV-REG-AS-03": "ATM strike K is not recorded; set it from the listing.",
    "XEV-REG-EU-03": "ATM strike K is not recorded; set it from the listing.",
    "XEV-REG-ME-05": "ATM strike K is not recorded; set it from the listing.",
    "XEV-REG-AS-06": "BTC 60 s BRTI average is not an integer, but the future's divider is 1: decide the contractual rounding to USD.",
    "XEV-STD-CRY-01": "As AS-06 (average of BRTI vs divider 1).",
    "XEV-STD-CRY-04": "ETH 60 s average vs divider 1: decide the rounding.",
    "XEV-REG-IND-03": "FBIL USD/INR is published to 4 decimals but the future's divider is 100; strict '>' on the finer value disagrees with the future's tick.",
    "XEV-REG-IND-04": "Official close has 2 decimals, the future's divider is 1.",
    "XEV-STD-FIN-01": "Official close has 2 decimals, the future's divider is 1.",
    "XEV-STD-FIN-02": "Official close has 2 decimals, the future's divider is 1.",
    "XEV-REG-IND-09": "LBMA PM price to 2 decimals vs divider 10.",
    "XEV-REG-IND-05": "Rainfall % of LPA: the futures band (1..1000 at divider 100) cannot hold a value near 90.",
    "XEV-REG-IND-10": "Seat counts (about 200) do not fit the futures band (1..10000 at divider 100): check the units.",
    "XEV-STD-ECO-06": "NFP in thousands vs a divider-100 band of 1..1000: check the units.",
    "XEV-REG-ME-03": "OPEC+ 'increase' is > 0 but the band starts at 1, so a cut or zero cannot be represented.",
    "XEV-STD-FIN-03": "Bracket 6,600 to 6,799.99 against an integer axis.",
    "XEV-STD-CLI-01": "Bracket width for B64.5 is not recorded.",
    "XEV-REG-AS-05": "Bracket width for B24 is not recorded.",
    "XEV-STD-CLI-02": "Precipitation >= 0.01 in: confirm trace and zero are outcomes at the band's lower edge.",
    "XEV-STD-COM-03": "Pays on close > prior close: needs the prior close as an input.",
    "XEV-STD-CRY-02": "Pays on settlement >= an opening reference fixed at listing.",
    "XEV-STD-ELE-01": "Chamber control 'per terms': needs the seat threshold and tie rules.",
    "XEV-STD-ELE-02": "Senate control: 50 seats plus the VP is ambiguous on a seat-count axis.",
    "XEV-REG-EU-04": "Election winner vs a vote-share future: no single threshold.",
}
STAY_ISOLATED = {
    "XEV-STD-COM-02": "Pays if ANY price in the month is >= the barrier: path dependent, not a function of the final value.",
    "XEV-STD-CRY-03": "Pays if the MIN over a window is <= the barrier: path dependent.",
    "XEV-STD-GEO-02": "Pays if ANY 7-day average in the window exceeds 60: path dependent.",
    "XEV-REG-ME-06": "US-Iran ceasefire has no matching future; the Brent leg is only a proxy hedge.",
    "XEV-STD-ELE-03": "Nomination contracts have no future.",
}

# Events whose only listing problem is the wording of settlement_source (same agency, different
# text). Optional file: aligns each future's text to its binary's. This edits contract terms, so
# product/legal must confirm it. Events with different RESOLUTION TIMES are not touched here.
ALIGN_SOURCE = [e for e, *_ in AUTO if e not in (
    "XEV-STD-ECO-02",   # already passes
    "XEV-STD-ECO-03", "XEV-STD-ECO-04", "XEV-REG-EU-02", "XEV-REG-IND-02",  # resolution times differ
)]

def ticks(strike, d):
    return int((D(strike) * d).to_integral_value())

def rule(strike, cmp_, d):
    s = ticks(strike, d)
    if cmp_ == ">":
        return {"type": "range", "lo": s + 1, "hi": None, "outcome_divider": d}
    if cmp_ == ">=":
        return {"type": "range", "lo": s, "hi": None, "outcome_divider": d}
    if cmp_ == "<":
        return {"type": "range", "lo": None, "hi": s - 1, "outcome_divider": d}
    raise ValueError(cmp_)

lines = [
    "-- PROPOSED cross-margin backfill (prerequisite fix 5). Generated by generate_backfill.py.",
    "-- Review services/migrations/proposals/backfill_report.md first. Nothing here changes any event's",
    "-- margin_model: every event stays ISOLATED. It only describes each contract's outcome axis so",
    "-- the shadow log (and later LIVE margin) can price it.",
    "BEGIN;",
]
for event, binary, strike, cmp_, d, step, note in AUTO:
    r = json.dumps(rule(strike, cmp_, d), separators=(",", ":"))
    lines += [
        f"-- {event}: {binary} YES iff outcome {cmp_} {strike} (axis divider {d}, step {step}; {note})",
        f"UPDATE refdata.events SET outcome_divider={d}, outcome_step={step} WHERE event_ticker='{event}';",
        f"UPDATE refdata.contracts SET payoff_rule='{r}'::jsonb, settlement_rule = COALESCE(settlement_rule,'{{}}'::jsonb) || '{{\"rounding_step\":{step}}}'::jsonb WHERE ticker='{binary}';",
        f"UPDATE refdata.contracts SET settlement_rule = COALESCE(settlement_rule,'{{}}'::jsonb) || '{{\"rounding_step\":{step}}}'::jsonb WHERE event_ticker='{event}' AND kind='SCALAR';",
    ]
lines.append("COMMIT;")
open("proposed_payoff_rules.sql", "w").write("\n".join(lines) + "\n")

align = ["-- OPTIONAL: align each future's settlement_source text with its binary's (same agency, wording only).",
         "-- Edits contract terms: confirm with product/legal before applying. See backfill_report.md.", "BEGIN;"]
for event in ALIGN_SOURCE:
    align.append(
        "UPDATE refdata.contracts f SET settlement_source = b.settlement_source "
        "FROM refdata.contracts b WHERE f.event_ticker = b.event_ticker AND f.kind = 'SCALAR' "
        f"AND b.kind = 'BINARY' AND f.event_ticker = '{event}';")
align.append("COMMIT;")
open("proposed_source_alignment.sql", "w").write("\n".join(align) + "\n")

report = ["# Cross margin backfill: proposal", "",
          "Generated by `generate_backfill.py` from the live contract terms read on 12 Oct 2026. "
          "Nothing has been applied to production.", "",
          f"## Proposed ({len(AUTO)} events)", "",
          "| Event | Binary | YES iff | Axis divider | Step | Note |", "|---|---|---|---|---|---|"]
for event, binary, strike, cmp_, d, step, note in AUTO:
    report.append(f"| {event} | {binary} | outcome {cmp_} {strike} (ticks {json.dumps(rule(strike, cmp_, d))}) | {d} | {step} | {note} |")
report += ["", "## What the listing validator says after the backfill (checked on a copy of the live reference data)", "",
           "Only `XEV-STD-ECO-02` passes as is. The validator also compares the two legs' settlement source text and expiry:", "",
           f"* **{len(ALIGN_SOURCE)} events** fail only because the future's `settlement_source` is worded differently from the binary's (for example `CPCB` vs `CPCB National AQI Bulletin`). `proposed_source_alignment.sql` aligns them; it edits contract terms, so product/legal should confirm.",
           "* **4 events** also have different `expected_resolution_at` on the two legs (`XEV-STD-ECO-03`, `XEV-STD-ECO-04`, `XEV-REG-EU-02`, `XEV-REG-IND-02`; the futures show placeholder-looking dates such as 2027-01-02, and one is already in the past). A pair that does not settle at the same time cannot share margin, so someone has to say which time is right.", ""]
report += ["", f"## Needs a decision ({len(DECISION)} events)", "", "| Event | Why |", "|---|---|"]
report += [f"| {k} | {v} |" for k, v in DECISION.items()]
report += ["", f"## Should stay ISOLATED ({len(STAY_ISOLATED)} events)", "", "| Event | Why |", "|---|---|"]
report += [f"| {k} | {v} |" for k, v in STAY_ISOLATED.items()]
open("backfill_report.md", "w").write("\n".join(report) + "\n")
print(len(AUTO), len(DECISION), len(STAY_ISOLATED), "total", len(AUTO) + len(DECISION) + len(STAY_ISOLATED))
