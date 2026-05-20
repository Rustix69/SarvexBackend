package m3svc

import (
	"context"
	"os"
	"testing"

	"github.com/jackc/pgx/v5/pgxpool"
	sarvexv1 "github.com/sarvex/proto/gen/go/sarvex/v1"
)

func testServer(t *testing.T) *ledgerServer {
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

	_, _ = pool.Exec(context.Background(), "TRUNCATE ledger.entries, ledger.transactions, ledger.hold_operations, ledger.holds, ledger.ledger_event_outbox RESTART IDENTITY CASCADE")
	_, _ = pool.Exec(context.Background(), "DELETE FROM ledger.accounts WHERE account_code LIKE 'LIAB:USER:%' OR account_code='ASSET:TEST:WALLET'")
	_, _ = pool.Exec(context.Background(), "INSERT INTO ledger.accounts (account_code, account_type, currency) VALUES ('ASSET:TEST:WALLET','ASSET','USDC') ON CONFLICT (account_code) DO NOTHING")

	return &ledgerServer{pg: pool}
}

func TestAdminDepositAndBalance(t *testing.T) {
	s := testServer(t)
	ctx := context.Background()
	_, err := s.AdminCreditDeposit(ctx, &sarvexv1.AdminCreditDepositRequest{UserId: "u_t1", AmountMicroUsdc: 100_000_000, Note: "seed"})
	if err != nil {
		t.Fatalf("AdminCreditDeposit: %v", err)
	}
	bal, err := s.GetBalance(ctx, &sarvexv1.GetBalanceRequest{UserId: "u_t1"})
	if err != nil {
		t.Fatalf("GetBalance: %v", err)
	}
	if bal.CashMicroUsdc != 100_000_000 || bal.HeldMicroUsdc != 0 {
		t.Fatalf("unexpected balance: %+v", bal)
	}
}

func TestPlaceHoldIdempotentAndRelease(t *testing.T) {
	s := testServer(t)
	ctx := context.Background()
	_, err := s.AdminCreditDeposit(ctx, &sarvexv1.AdminCreditDepositRequest{UserId: "u_t2", AmountMicroUsdc: 200_000_000, Note: "seed"})
	if err != nil {
		t.Fatalf("AdminCreditDeposit seed: %v", err)
	}

	p1, err := s.PlaceHold(ctx, &sarvexv1.PlaceHoldRequest{IdempotencyKey: "hold-1", UserId: "u_t2", AmountMicroUsdc: 50_000_000, Reason: "order"})
	if err != nil {
		t.Fatalf("PlaceHold first: %v", err)
	}
	p2, err := s.PlaceHold(ctx, &sarvexv1.PlaceHoldRequest{IdempotencyKey: "hold-1", UserId: "u_t2", AmountMicroUsdc: 50_000_000, Reason: "order"})
	if err != nil {
		t.Fatalf("PlaceHold second: %v", err)
	}
	if p1.HoldId != p2.HoldId {
		t.Fatalf("idempotency hold mismatch: %s vs %s", p1.HoldId, p2.HoldId)
	}

	_, err = s.ReleaseHold(ctx, &sarvexv1.ReleaseHoldRequest{IdempotencyKey: "rel-1", HoldId: p1.HoldId, AmountMicroUsdc: 20_000_000, ReasonCode: "CANCEL"})
	if err != nil {
		t.Fatalf("ReleaseHold: %v", err)
	}
	bal, err := s.GetBalance(ctx, &sarvexv1.GetBalanceRequest{UserId: "u_t2"})
	if err != nil {
		t.Fatalf("GetBalance: %v", err)
	}
	if bal.HeldMicroUsdc != 30_000_000 {
		t.Fatalf("expected held=30000000 got=%d", bal.HeldMicroUsdc)
	}
}

func TestPlaceHoldInsufficientFunds(t *testing.T) {
	s := testServer(t)
	ctx := context.Background()
	_, err := s.PlaceHold(ctx, &sarvexv1.PlaceHoldRequest{IdempotencyKey: "hold-low", UserId: "u_t3", AmountMicroUsdc: 9_999_999, Reason: "order"})
	if err == nil {
		t.Fatal("expected insufficient funds error")
	}
}
