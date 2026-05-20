//go:build e2e

package e2e

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"os"
	"testing"
	"time"

	"github.com/jackc/pgx/v5/pgxpool"
	sarvexv1 "github.com/sarvex/proto/gen/go/sarvex/v1"
	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials/insecure"
)

func TestMVPTradeCloseResolveSettle(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()

	pg := mustPool(t, ctx)
	resetTickerState(t, ctx, pg)

	ledger := sarvexv1.NewLedgerClient(mustDial(t, env("LEDGER_ADDR", "localhost:15062")))
	refdata := sarvexv1.NewRefDataClient(mustDial(t, env("REFDATA_ADDR", "localhost:15061")))
	oracle := sarvexv1.NewOracleClient(mustDial(t, env("ORACLE_ADDR", "localhost:15067")))
	settlement := sarvexv1.NewSettlementClient(mustDial(t, env("SETTLEMENT_ADDR", "localhost:15068")))
	matching := sarvexv1.NewMatchingEngineClient(mustDial(t, env("MATCHING_ADDR", "localhost:15064")))
	if _, err := matching.AddBook(ctx, &sarvexv1.AddBookRequest{Ticker: "RBI-JUN26-CUT25", Kind: sarvexv1.ContractKind_CONTRACT_KIND_BINARY, TickSize: 1, MinPriceTicks: 1, MaxPriceTicks: 99}); err != nil {
		t.Fatalf("reset matching book: %v", err)
	}

	for _, user := range []string{"u_mm_1", "u_retail_1"} {
		_, err := ledger.AdminCreditDeposit(ctx, &sarvexv1.AdminCreditDepositRequest{UserId: user, AmountMicroUsdc: 100_000_000, Note: "e2e"})
		if err != nil {
			t.Fatalf("deposit %s: %v", user, err)
		}
	}

	mmToken := login(t, "u_mm_1")
	retailToken := login(t, "u_retail_1")
	suffix := fmt.Sprintf("%d", time.Now().UnixNano())
	maker := submitOrder(t, mmToken, "e2e-maker-"+suffix, "SELL")
	if maker["reject_code"] != nil && maker["reject_code"] != "" {
		t.Fatalf("maker rejected: %#v", maker)
	}
	taker := submitOrder(t, retailToken, "e2e-taker-"+suffix, "BUY")
	if taker["reject_code"] != nil && taker["reject_code"] != "" {
		t.Fatalf("taker rejected: %#v", taker)
	}

	waitForCount(t, ctx, pg, `SELECT COUNT(*) FROM orders.fills WHERE ticker='RBI-JUN26-CUT25'`, 1)
	waitForCount(t, ctx, pg, `SELECT COUNT(*) FROM orders.fills WHERE ticker='RBI-JUN26-CUT25' AND ledger_post_status='POSTED'`, 1)

	_, err := refdata.TransitionState(ctx, &sarvexv1.TransitionStateRequest{
		Ticker: "RBI-JUN26-CUT25", NewState: sarvexv1.ContractState_CONTRACT_STATE_CLOSED, Reason: "e2e close",
	})
	if err != nil {
		t.Fatalf("close contract: %v", err)
	}
	_, err = oracle.AdminForceResolution(ctx, &sarvexv1.AdminForceResolutionRequest{
		EventTicker: "RBI-JUN26", CategoricalValue: "YES", AdminUserId: "u_admin", Justification: "e2e",
	})
	if err != nil {
		t.Fatalf("force resolution: %v", err)
	}
	resp, err := settlement.SettleContract(ctx, &sarvexv1.SettleContractRequest{Ticker: "RBI-JUN26-CUT25", EventTicker: "RBI-JUN26"})
	if err != nil {
		t.Fatalf("settle: %v", err)
	}
	if resp.GetPositionsSettled() == 0 || resp.GetTotalPayoutMicroUsdc() == 0 {
		t.Fatalf("unexpected settlement result: %#v", resp)
	}
	var state string
	if err := pg.QueryRow(ctx, `SELECT state::text FROM refdata.contracts WHERE ticker='RBI-JUN26-CUT25'`).Scan(&state); err != nil {
		t.Fatal(err)
	}
	if state != "SETTLED" {
		t.Fatalf("expected SETTLED, got %s", state)
	}
}

func mustPool(t *testing.T, ctx context.Context) *pgxpool.Pool {
	t.Helper()
	pool, err := pgxpool.New(ctx, env("TEST_POSTGRES_DSN", "postgres://sarvaex:sarvaex@localhost:15432/sarvaex?sslmode=disable"))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(pool.Close)
	return pool
}

func mustDial(t *testing.T, addr string) *grpc.ClientConn {
	t.Helper()
	conn, err := grpc.NewClient(addr, grpc.WithTransportCredentials(insecure.NewCredentials()))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = conn.Close() })
	return conn
}

func resetTickerState(t *testing.T, ctx context.Context, pg *pgxpool.Pool) {
	t.Helper()
	_, err := pg.Exec(ctx, `
DELETE FROM settlement.settlement_payouts WHERE ticker='RBI-JUN26-CUT25';
DELETE FROM settlement.settlements WHERE ticker='RBI-JUN26-CUT25';
DELETE FROM oracle.attestations WHERE event_ticker='RBI-JUN26';
DELETE FROM oracle.resolutions WHERE event_ticker='RBI-JUN26';
DELETE FROM position.position_history WHERE ticker='RBI-JUN26-CUT25';
DELETE FROM position.applied_fills WHERE ticker='RBI-JUN26-CUT25';
DELETE FROM position.positions WHERE ticker='RBI-JUN26-CUT25';
DELETE FROM position.consumer_offsets WHERE stream_name='exec.fills';
DELETE FROM orders.fill_posting_outbox WHERE fill_id IN (SELECT fill_id FROM orders.fills WHERE ticker='RBI-JUN26-CUT25');
DELETE FROM orders.fills WHERE ticker='RBI-JUN26-CUT25';
DELETE FROM orders.orders WHERE ticker='RBI-JUN26-CUT25';
UPDATE refdata.contracts SET state='OPEN', close_global_seq=NULL, close_at=NULL WHERE ticker='RBI-JUN26-CUT25';
`)
	if err != nil {
		t.Fatal(err)
	}
}

func login(t *testing.T, userID string) string {
	t.Helper()
	body := postJSON(t, "/v1/auth/login", "", "", map[string]any{"user_id": userID})
	tok, _ := body["token"].(string)
	if tok == "" {
		t.Fatalf("missing token in %#v", body)
	}
	return tok
}

func submitOrder(t *testing.T, token, clientOrderID, action string) map[string]any {
	t.Helper()
	return postJSON(t, "/v1/orders", token, clientOrderID, map[string]any{
		"client_order_id": clientOrderID,
		"ticker":          "RBI-JUN26-CUT25",
		"side":            "YES",
		"action":          action,
		"price_ticks":     50,
		"count":           10,
		"tif":             "GTC",
	})
}

func postJSON(t *testing.T, path, token, idem string, payload map[string]any) map[string]any {
	t.Helper()
	b, _ := json.Marshal(payload)
	req, err := http.NewRequest(http.MethodPost, env("GW_REST_URL", "http://localhost:19080")+path, bytes.NewReader(b))
	if err != nil {
		t.Fatal(err)
	}
	req.Header.Set("Content-Type", "application/json")
	if token != "" {
		req.Header.Set("Authorization", "Bearer "+token)
	}
	if idem != "" {
		req.Header.Set("Idempotency-Key", idem)
	}
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		t.Fatal(err)
	}
	defer resp.Body.Close()
	var out map[string]any
	_ = json.NewDecoder(resp.Body).Decode(&out)
	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		t.Fatalf("POST %s status=%d body=%#v", path, resp.StatusCode, out)
	}
	return out
}

func waitForCount(t *testing.T, ctx context.Context, pg *pgxpool.Pool, q string, want int64) {
	t.Helper()
	deadline := time.Now().Add(5 * time.Second)
	for time.Now().Before(deadline) {
		var got int64
		if err := pg.QueryRow(ctx, q).Scan(&got); err == nil && got >= want {
			return
		}
		time.Sleep(100 * time.Millisecond)
	}
	var got int64
	_ = pg.QueryRow(ctx, q).Scan(&got)
	t.Fatalf("timeout waiting for count: got %d want %d query %s", got, want, q)
}

func env(k, def string) string {
	if v := os.Getenv(k); v != "" {
		return v
	}
	return def
}

func Example() {
	fmt.Println("run with: go test -tags=e2e ./pkg/e2e")
}
