package m3svc

import (
	"context"
	"fmt"
	"math"
	"strings"
	"time"

	"github.com/jackc/pgx/v5"
	sarvexv1 "github.com/sarvex/proto/gen/go/sarvex/v1"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
	"google.golang.org/protobuf/types/known/timestamppb"
)

func (s *settlementServer) SettleContract(ctx context.Context, req *sarvexv1.SettleContractRequest) (*sarvexv1.SettlementResult, error) {
	if strings.TrimSpace(req.GetTicker()) == "" || strings.TrimSpace(req.GetEventTicker()) == "" {
		return nil, status.Error(codes.InvalidArgument, "ticker and event_ticker required")
	}
	type contractRow struct {
		kind, state string
		closeSeq    int64
		lower       int64
		upper       int64
		multiplier  int64
	}
	var c contractRow
	err := s.pg.QueryRow(ctx, `SELECT kind::text, state::text, COALESCE(close_global_seq,0), COALESCE(lower_bound_ticks,0), COALESCE(upper_bound_ticks,0), COALESCE(multiplier_micro_usdc,0)
FROM refdata.contracts WHERE ticker=$1`, req.GetTicker()).Scan(&c.kind, &c.state, &c.closeSeq, &c.lower, &c.upper, &c.multiplier)
	if err != nil {
		return nil, mapPgErr(err)
	}
	if c.closeSeq <= 0 {
		return nil, status.Error(codes.FailedPrecondition, "close_global_seq missing")
	}
	if c.state != "CLOSED" && c.state != "RESOLVING" && c.state != "SETTLED" {
		return nil, status.Error(codes.FailedPrecondition, "contract must be closed before settlement")
	}
	var unresolved int64
	_ = s.pg.QueryRow(ctx, `SELECT COUNT(1) FROM orders.fills WHERE ticker=$1 AND global_seq <= $2 AND ledger_post_status != 'POSTED'`, req.GetTicker(), c.closeSeq).Scan(&unresolved)
	if unresolved > 0 {
		return nil, status.Error(codes.FailedPrecondition, "fills not fully posted to ledger through close_global_seq")
	}
	var activeOrders int64
	_ = s.pg.QueryRow(ctx, `SELECT COUNT(1) FROM orders.orders WHERE ticker=$1 AND status IN ('PENDING','OPEN','PARTIAL')`, req.GetTicker()).Scan(&activeOrders)
	if activeOrders > 0 {
		return nil, status.Error(codes.FailedPrecondition, "open orders remain; hold releases not complete")
	}
	var posAppliedSeq int64
	_ = s.pg.QueryRow(ctx, `SELECT COALESCE(last_global_seq,0) FROM position.consumer_offsets WHERE stream_name='exec.fills'`).Scan(&posAppliedSeq)
	if posAppliedSeq < c.closeSeq {
		return nil, status.Error(codes.FailedPrecondition, "position consumer has not caught up through close_global_seq")
	}

	var numeric int64
	var categorical string
	err = s.pg.QueryRow(ctx, `SELECT COALESCE(numeric_value,0), COALESCE(categorical_value,'') FROM oracle.resolutions WHERE event_ticker=$1 AND status='FINALIZED'`, req.GetEventTicker()).Scan(&numeric, &categorical)
	if err != nil {
		return nil, status.Error(codes.FailedPrecondition, "finalized oracle resolution missing")
	}

	_, _ = s.pg.Exec(ctx, `UPDATE refdata.contracts SET state='RESOLVING'::refdata.contract_state, updated_at=now() WHERE ticker=$1 AND state='CLOSED'::refdata.contract_state`, req.GetTicker())

	tx, err := s.pg.BeginTx(ctx, pgx.TxOptions{})
	if err != nil {
		return nil, mapPgErr(err)
	}
	defer tx.Rollback(ctx)

	_, err = tx.Exec(ctx, `INSERT INTO settlement.settlements
(ticker, event_ticker, numeric_value, categorical_value, winner_payout_per_contract_micro_usdc, close_global_seq, positions_source_global_seq, status, started_at)
VALUES ($1,$2,$3,$4,0,$5,$6,'IN_PROGRESS',now())
ON CONFLICT (ticker) DO UPDATE SET event_ticker=EXCLUDED.event_ticker, numeric_value=EXCLUDED.numeric_value, categorical_value=EXCLUDED.categorical_value,
close_global_seq=EXCLUDED.close_global_seq, positions_source_global_seq=EXCLUDED.positions_source_global_seq, status='IN_PROGRESS', started_at=COALESCE(settlement.settlements.started_at, now())`,
		req.GetTicker(), req.GetEventTicker(), numeric, categorical, c.closeSeq, posAppliedSeq)
	if err != nil {
		return nil, mapPgErr(err)
	}

	rows, err := tx.Query(ctx, `SELECT user_id, net_qty FROM position.positions WHERE ticker=$1 AND net_qty != 0`, req.GetTicker())
	if err != nil {
		return nil, mapPgErr(err)
	}
	defer rows.Close()
	ledger := &ledgerServer{pg: s.pg}
	var totalPayout int64
	var count int32
	for rows.Next() {
		var userID string
		var netQty int64
		if err := rows.Scan(&userID, &netQty); err != nil {
			continue
		}
		payout := computePayoutMicro(c.kind, netQty, numeric, categorical, c.lower, c.upper, c.multiplier)
		if payout <= 0 {
			continue
		}
		idem := fmt.Sprintf("settlement:%s:%s", req.GetTicker(), userID)
		dest := fmt.Sprintf("LIAB:USER:%s:CASH", userID)
		_, _ = ledger.PostTransaction(ctx, &sarvexv1.PostTransactionRequest{
			IdempotencyKey: idem,
			ReasonCode:     "SETTLEMENT_PAYOUT",
			Entries: []*sarvexv1.LedgerEntry{
				{AccountCode: "LIAB:HOUSE:UNSETTLED_TRADES:" + req.GetTicker(), Direction: "DR", AmountMicroUsdc: payout, Memo: "settlement payout"},
				{AccountCode: dest, Direction: "CR", AmountMicroUsdc: payout, Memo: "settlement payout"},
			},
		})
		_, _ = tx.Exec(ctx, `INSERT INTO settlement.settlement_payouts (ticker,user_id,position_qty,payout_micro_usdc,idempotency_key,status,posted_at)
VALUES ($1,$2,$3,$4,$5,'POSTED',now())
ON CONFLICT (idempotency_key) DO UPDATE SET payout_micro_usdc=EXCLUDED.payout_micro_usdc, status='POSTED'`,
			req.GetTicker(), userID, netQty, payout, idem)
		totalPayout += payout
		count++
	}
	roundingSweepID := fmt.Sprintf("settlement:%s:rounding_sweep", req.GetTicker())
	_, _ = ledger.PostTransaction(ctx, &sarvexv1.PostTransactionRequest{
		IdempotencyKey: roundingSweepID,
		ReasonCode:     "SETTLEMENT_ROUNDING_SWEEP",
		Entries: []*sarvexv1.LedgerEntry{
			{AccountCode: "LIAB:HOUSE:UNSETTLED_TRADES:" + req.GetTicker(), Direction: "DR", AmountMicroUsdc: 0, Memo: "rounding sweep"},
			{AccountCode: "REVENUE:SETTLEMENT_ROUNDING", Direction: "CR", AmountMicroUsdc: 0, Memo: "rounding sweep"},
		},
	})

	_, err = tx.Exec(ctx, `UPDATE settlement.settlements SET
winner_payout_per_contract_micro_usdc=$2,total_payout_micro_usdc=$3,positions_settled=$4,status='COMPLETED',completed_at=now()
WHERE ticker=$1`, req.GetTicker(), winnerPayoutPerContract(c.kind, numeric, categorical, c.lower, c.upper, c.multiplier), totalPayout, count)
	if err != nil {
		return nil, mapPgErr(err)
	}
	if err := tx.Commit(ctx); err != nil {
		return nil, mapPgErr(err)
	}
	_, _ = s.pg.Exec(ctx, `UPDATE refdata.contracts SET state='SETTLED'::refdata.contract_state, updated_at=now() WHERE ticker=$1`, req.GetTicker())
	return s.GetSettlement(ctx, &sarvexv1.GetSettlementRequest{Ticker: req.GetTicker()})
}

func (s *settlementServer) GetSettlement(ctx context.Context, req *sarvexv1.GetSettlementRequest) (*sarvexv1.SettlementResult, error) {
	var out sarvexv1.SettlementResult
	var completed *time.Time
	err := s.pg.QueryRow(ctx, `SELECT ticker, completed_at, winner_payout_per_contract_micro_usdc, total_payout_micro_usdc, positions_settled
FROM settlement.settlements WHERE ticker=$1`, req.GetTicker()).
		Scan(&out.Ticker, &completed, &out.WinnerPayoutPerContractMicroUsdc, &out.TotalPayoutMicroUsdc, &out.PositionsSettled)
	if err != nil {
		return nil, mapPgErr(err)
	}
	if completed != nil {
		out.SettledAt = timestamppb.New(*completed)
	}
	return &out, nil
}

func winnerPayoutPerContract(kind string, numeric int64, categorical string, lower, upper, multiplier int64) int64 {
	if kind == "BINARY" {
		return 1_000_000
	}
	if multiplier <= 0 {
		return 0
	}
	v := clamp(numeric, lower, upper)
	return (v - lower) * multiplier
}

func computePayoutMicro(kind string, netQty int64, numeric int64, categorical string, lower, upper, multiplier int64) int64 {
	if netQty == 0 {
		return 0
	}
	if kind == "BINARY" {
		winYes := strings.EqualFold(categorical, "YES") || numeric > 0
		if winYes && netQty > 0 {
			return netQty * 1_000_000
		}
		if !winYes && netQty < 0 {
			return -netQty * 1_000_000
		}
		return 0
	}
	if multiplier <= 0 {
		return 0
	}
	v := clamp(numeric, lower, upper)
	if netQty > 0 {
		return netQty * (v-lower) * multiplier
	}
	return (-netQty) * (upper-v) * multiplier
}

func clamp(v, lo, hi int64) int64 {
	return int64(math.Max(float64(lo), math.Min(float64(hi), float64(v))))
}
