package m3svc

import (
	"context"
	"os"
	"testing"
	"time"

	"github.com/jackc/pgx/v5/pgxpool"
	sarvexv1 "github.com/sarvex/proto/gen/go/sarvex/v1"
	"google.golang.org/protobuf/types/known/structpb"
	"google.golang.org/protobuf/types/known/timestamppb"
)

func testPool(t *testing.T) *pgxpool.Pool {
	t.Helper()
	dsn := os.Getenv("TEST_POSTGRES_DSN")
	if dsn == "" {
		dsn = "postgres://sarvaex:sarvaex@localhost:15432/sarvaex?sslmode=disable"
	}
	pool, err := pgxpool.New(context.Background(), dsn)
	if err != nil {
		t.Skipf("postgres unavailable: %v", err)
	}
	if err := pool.Ping(context.Background()); err != nil {
		t.Skipf("postgres ping failed: %v", err)
	}
	t.Cleanup(pool.Close)
	return pool
}

func seedSeriesEvent(t *testing.T, pool *pgxpool.Pool, series, event string) {
	t.Helper()
	_, err := pool.Exec(context.Background(),
		`INSERT INTO refdata.series (series_ticker, title, description)
VALUES ($1,$2,$3) ON CONFLICT (series_ticker) DO NOTHING`,
		series, "Test Series", "Test",
	)
	if err != nil {
		t.Fatalf("seed series: %v", err)
	}
	_, err = pool.Exec(context.Background(),
		`INSERT INTO refdata.events (event_ticker, series_ticker, title, description, expected_resolution_at)
VALUES ($1,$2,$3,$4,$5) ON CONFLICT (event_ticker) DO NOTHING`,
		event, series, "Test Event", "Test", time.Now().UTC().Add(24*time.Hour),
	)
	if err != nil {
		t.Fatalf("seed event: %v", err)
	}
}

func TestRefDataUpsertGetTransitionList(t *testing.T) {
	pool := testPool(t)
	srv := &refDataServer{pg: pool}
	ctx := context.Background()

	series := "TEST-SERIES-M5"
	event := "TEST-EVENT-M5"
	ticker := "TEST-M5-BIN"
	seedSeriesEvent(t, pool, series, event)

	rule, _ := structpb.NewStruct(map[string]any{"type": "categorical_equals", "yes_values": []any{"YES"}})
	contract := &sarvexv1.Contract{
		Ticker:               ticker,
		EventTicker:          event,
		SeriesTicker:         series,
		Kind:                 sarvexv1.ContractKind_CONTRACT_KIND_BINARY,
		Question:             "Will test pass?",
		TickSize:             1,
		MinPriceTicks:        1,
		MaxPriceTicks:        99,
		MaxOrderSize:         1000,
		PositionLimitPerUser: 5000,
		State:                sarvexv1.ContractState_CONTRACT_STATE_OPEN,
		ExpectedResolutionAt: timestamppb.New(time.Now().UTC().Add(48 * time.Hour)),
		SettlementSource:     "test",
		OraclePolicy:         "ADMIN",
		SettlementRule:       rule,
	}

	up, err := srv.UpsertContract(ctx, &sarvexv1.UpsertContractRequest{Contract: contract})
	if err != nil {
		t.Fatalf("UpsertContract: %v", err)
	}
	if up.GetTicker() != ticker {
		t.Fatalf("unexpected ticker: %s", up.GetTicker())
	}

	got, err := srv.GetContract(ctx, &sarvexv1.GetContractRequest{Ticker: ticker})
	if err != nil {
		t.Fatalf("GetContract: %v", err)
	}
	if got.GetState() != sarvexv1.ContractState_CONTRACT_STATE_OPEN {
		t.Fatalf("unexpected state: %v", got.GetState())
	}

	_, err = srv.TransitionState(ctx, &sarvexv1.TransitionStateRequest{Ticker: ticker, NewState: sarvexv1.ContractState_CONTRACT_STATE_HALTED, Reason: "test"})
	if err != nil {
		t.Fatalf("TransitionState: %v", err)
	}
	got, err = srv.GetContract(ctx, &sarvexv1.GetContractRequest{Ticker: ticker})
	if err != nil {
		t.Fatalf("GetContract after transition: %v", err)
	}
	if got.GetState() != sarvexv1.ContractState_CONTRACT_STATE_HALTED {
		t.Fatalf("expected HALTED got %v", got.GetState())
	}

	list, err := srv.ListContracts(ctx, &sarvexv1.ListContractsRequest{SeriesTicker: series, Limit: 10})
	if err != nil {
		t.Fatalf("ListContracts: %v", err)
	}
	if len(list.GetContracts()) == 0 {
		t.Fatal("expected at least one contract")
	}
}

func TestRiskPreTradeCheckAndLimits(t *testing.T) {
	pool := testPool(t)
	riskSrv := &riskServer{pg: pool}
	ctx := context.Background()

	sfx := itoa(int(time.Now().UnixNano() % 1000000))
	series := "TEST-SERIES-M5-RISK-" + sfx
	event := "TEST-EVENT-M5-RISK-" + sfx
	ticker := "TEST-M5-RISK-" + sfx
	user := "u_m5_risk_" + sfx
	seedSeriesEvent(t, pool, series, event)

	_, err := pool.Exec(ctx,
		`INSERT INTO refdata.contracts (
ticker, event_ticker, series_ticker, kind, question,
tick_size, min_price_ticks, max_price_ticks,
max_order_size, position_limit_per_user, state, expected_resolution_at,
settlement_source, oracle_policy, settlement_rule
) VALUES ($1,$2,$3,'BINARY',$4,1,1,99,1000,100,'OPEN',$5,'test','ADMIN','{}'::jsonb)
ON CONFLICT (ticker) DO UPDATE SET state='OPEN', position_limit_per_user=100`,
		ticker, event, series, "Risk test", time.Now().UTC().Add(48*time.Hour),
	)
	if err != nil {
		t.Fatalf("seed contract: %v", err)
	}
	_, err = pool.Exec(ctx,
		`INSERT INTO risk.user_limits (user_id, kyc_tier, max_order_size_micro_usdc, daily_loss_limit_micro_usdc)
VALUES ($1,1,1000000000,10000000000)
ON CONFLICT (user_id) DO UPDATE SET max_order_size_micro_usdc=EXCLUDED.max_order_size_micro_usdc`,
		user,
	)
	if err != nil {
		t.Fatalf("seed user limits: %v", err)
	}

	resp, err := riskSrv.PreTradeCheck(ctx, &sarvexv1.PreTradeCheckRequest{
		UserId:     user,
		Ticker:     ticker,
		Side:       sarvexv1.Side_SIDE_YES,
		Action:     sarvexv1.Action_ACTION_BUY,
		PriceTicks: 50,
		Count:      10,
	})
	if err != nil {
		t.Fatalf("PreTradeCheck: %v", err)
	}
	if !resp.GetApproved() {
		t.Fatalf("expected approved, reject=%s", resp.GetRejectCode())
	}
	if resp.GetRequiredHoldMicroUsdc() != 50*10*10000 {
		t.Fatalf("unexpected required hold: %d", resp.GetRequiredHoldMicroUsdc())
	}

	_, err = riskSrv.UpdateUserLimits(ctx, &sarvexv1.UpdateUserLimitsRequest{
		Limits: &sarvexv1.UserLimits{
			UserId:                   user,
			KycTier:                  1,
			MaxOrderSizeMicroUsdc:    1000000000,
			DailyLossLimitMicroUsdc:  10000000000,
			PerContractPositionLimit: map[string]int64{ticker: 5},
		},
	})
	if err != nil {
		t.Fatalf("UpdateUserLimits: %v", err)
	}

	resp, err = riskSrv.PreTradeCheck(ctx, &sarvexv1.PreTradeCheckRequest{
		UserId:     user,
		Ticker:     ticker,
		Side:       sarvexv1.Side_SIDE_YES,
		Action:     sarvexv1.Action_ACTION_BUY,
		PriceTicks: 50,
		Count:      10,
	})
	if err != nil {
		t.Fatalf("PreTradeCheck with tightened limit: %v", err)
	}
	if resp.GetApproved() || resp.GetRejectCode() != "POSITION_LIMIT_EXCEEDED" {
		t.Fatalf("expected position reject, got approved=%v code=%s", resp.GetApproved(), resp.GetRejectCode())
	}
}
