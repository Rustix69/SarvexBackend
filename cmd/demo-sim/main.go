package main

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"log"
	"math/rand"
	"net/http"
	"os"
	"strings"
	"time"

	"github.com/jackc/pgx/v5/pgxpool"
	sarvexv1 "github.com/sarvex/proto/gen/go/sarvex/v1"
	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials/insecure"
)

type config struct {
	restURL      string
	ledgerAddr   string
	matchingAddr string
	pgDSN        string
	ticker       string
	users        int
	rounds       int
	interval     time.Duration
	fundUSDC     int64
	seedUsers    bool
	ensureOpen   bool
	resetBook    bool
	continuous   bool
	initialBook  bool
}

type bot struct {
	userID string
	token  string
}

type loginResp struct {
	Token string `json:"token"`
}

type errorResp struct {
	Error map[string]any `json:"error"`
}

func main() {
	cfg := parseFlags()
	ctx := context.Background()
	rng := rand.New(rand.NewSource(time.Now().UnixNano()))

	if cfg.seedUsers {
		if err := seedDemoUsers(ctx, cfg); err != nil {
			log.Fatalf("seed demo users: %v", err)
		}
	}
	if cfg.ensureOpen {
		reopened, err := ensureContractOpen(ctx, cfg)
		if err != nil {
			log.Fatalf("ensure contract open: %v", err)
		}
		if reopened {
			if err := reopenMatchingBook(ctx, cfg); err != nil {
				log.Fatalf("reopen matching book: %v", err)
			}
		}
	}
	if cfg.resetBook {
		if err := reopenMatchingBook(ctx, cfg); err != nil {
			log.Fatalf("reset matching book: %v", err)
		}
	}

	ledger, closeLedger, err := newLedgerClient(cfg.ledgerAddr)
	if err != nil {
		log.Fatalf("ledger connect: %v", err)
	}
	defer closeLedger()

	bots := make([]bot, 0, cfg.users)
	for i := 1; i <= cfg.users; i++ {
		userID := fmt.Sprintf("u_sim_%03d", i)
		if cfg.fundUSDC > 0 {
			_, err := ledger.AdminCreditDeposit(ctx, &sarvexv1.AdminCreditDepositRequest{
				UserId:          userID,
				AmountMicroUsdc: cfg.fundUSDC * 1_000_000,
				Note:            "demo simulator funding",
			})
			if err != nil {
				log.Fatalf("fund %s: %v", userID, err)
			}
		}
		tok, err := login(cfg.restURL, userID)
		if err != nil {
			log.Fatalf("login %s: %v", userID, err)
		}
		bots = append(bots, bot{userID: userID, token: tok})
	}

	log.Printf("simulator ready ticker=%s bots=%d rounds=%d continuous=%v", cfg.ticker, len(bots), cfg.rounds, cfg.continuous)
	if cfg.initialBook {
		seedInitialBook(ctx, cfg, bots, rng)
	}

	round := 0
	for {
		round++
		runRound(ctx, cfg, bots, rng, round)
		if !cfg.continuous && round >= cfg.rounds {
			break
		}
		time.Sleep(cfg.interval)
	}
	if !cfg.continuous && cfg.initialBook {
		seedInitialBook(ctx, cfg, bots, rng)
	}
	log.Printf("simulator complete ticker=%s rounds=%d", cfg.ticker, round)
}

func parseFlags() config {
	var cfg config
	flag.StringVar(&cfg.restURL, "rest-url", env("GW_REST_URL", "http://localhost:19080"), "REST gateway base URL")
	flag.StringVar(&cfg.ledgerAddr, "ledger-addr", env("LEDGER_ADDR", "localhost:15062"), "ledger gRPC address")
	flag.StringVar(&cfg.matchingAddr, "matching-addr", env("MATCHING_ADDR", "localhost:15064"), "matching engine gRPC address")
	flag.StringVar(&cfg.pgDSN, "pg-dsn", env("POSTGRES_DSN", env("TEST_POSTGRES_DSN", "postgres://sarvaex:sarvaex@localhost:15432/sarvaex?sslmode=disable")), "Postgres DSN for demo user/limit seeding")
	flag.StringVar(&cfg.ticker, "ticker", env("DEMO_TICKER", "RBI-JUN26-CUT25"), "contract ticker to simulate")
	flag.IntVar(&cfg.users, "users", 40, "number of simulated users")
	flag.IntVar(&cfg.rounds, "rounds", 20, "number of simulation rounds when not continuous")
	flag.DurationVar(&cfg.interval, "interval", 750*time.Millisecond, "delay between rounds")
	flag.Int64Var(&cfg.fundUSDC, "fund-usdc", 100_000, "USDC to credit to each simulated user; set 0 to skip funding")
	flag.BoolVar(&cfg.seedUsers, "seed-users", true, "insert demo simulator users and risk limits")
	flag.BoolVar(&cfg.ensureOpen, "ensure-open", true, "reopen the demo contract if a previous demo settled/closed it")
	flag.BoolVar(&cfg.resetBook, "reset-book", false, "reset/reopen the in-memory matching book before simulating")
	flag.BoolVar(&cfg.continuous, "continuous", false, "run until interrupted")
	flag.BoolVar(&cfg.initialBook, "initial-book", true, "seed passive book depth before trade rounds")
	flag.Parse()
	if cfg.users < 4 {
		cfg.users = 4
	}
	if cfg.rounds < 1 {
		cfg.rounds = 1
	}
	cfg.restURL = strings.TrimRight(cfg.restURL, "/")
	return cfg
}

func ensureContractOpen(ctx context.Context, cfg config) (bool, error) {
	pool, err := pgxpool.New(ctx, cfg.pgDSN)
	if err != nil {
		return false, err
	}
	defer pool.Close()
	cmd, err := pool.Exec(ctx, `UPDATE refdata.contracts
SET state='OPEN', close_global_seq=NULL, updated_at=now()
WHERE ticker=$1 AND state <> 'OPEN'`, cfg.ticker)
	if err != nil {
		return false, err
	}
	if cmd.RowsAffected() > 0 {
		log.Printf("reopened demo contract ticker=%s", cfg.ticker)
	}
	return cmd.RowsAffected() > 0, nil
}

func reopenMatchingBook(ctx context.Context, cfg config) error {
	conn, err := grpc.NewClient(cfg.matchingAddr, grpc.WithTransportCredentials(insecure.NewCredentials()))
	if err != nil {
		return err
	}
	defer conn.Close()
	_, err = sarvexv1.NewMatchingEngineClient(conn).AddBook(ctx, &sarvexv1.AddBookRequest{
		Ticker:        cfg.ticker,
		Kind:          sarvexv1.ContractKind_CONTRACT_KIND_BINARY,
		TickSize:      1,
		MinPriceTicks: 1,
		MaxPriceTicks: 99,
	})
	if err == nil {
		log.Printf("reopened matching book ticker=%s", cfg.ticker)
	}
	return err
}

func seedDemoUsers(ctx context.Context, cfg config) error {
	pool, err := pgxpool.New(ctx, cfg.pgDSN)
	if err != nil {
		return err
	}
	defer pool.Close()

	for i := 1; i <= cfg.users; i++ {
		userID := fmt.Sprintf("u_sim_%03d", i)
		email := fmt.Sprintf("sim%03d@demo.sarvex.com", i)
		display := fmt.Sprintf("Sim Trader %03d", i)
		isMM := i%5 == 0
		kycTier := int32(1)
		if isMM {
			kycTier = 2
		}
		_, err := pool.Exec(ctx, `INSERT INTO users.users (user_id, email, display_name, password_hash, kyc_tier, is_admin, is_mm)
VALUES ($1,$2,$3,$4,$5,false,$6)
ON CONFLICT (user_id) DO UPDATE SET email=EXCLUDED.email, display_name=EXCLUDED.display_name, kyc_tier=EXCLUDED.kyc_tier, is_mm=EXCLUDED.is_mm`,
			userID, email, display, "$2a$10$N9qo8uLOickgx2ZMRZoMyeIjZAgcfl7p92ldGxad68LJZdL17lhWy", kycTier, isMM)
		if err != nil {
			return err
		}
		_, err = pool.Exec(ctx, `INSERT INTO risk.user_limits (user_id, kyc_tier, max_order_size_micro_usdc, daily_loss_limit_micro_usdc, orders_per_second_limit)
VALUES ($1,$2,$3,$4,$5)
ON CONFLICT (user_id) DO UPDATE SET kyc_tier=EXCLUDED.kyc_tier, max_order_size_micro_usdc=EXCLUDED.max_order_size_micro_usdc, daily_loss_limit_micro_usdc=EXCLUDED.daily_loss_limit_micro_usdc, orders_per_second_limit=EXCLUDED.orders_per_second_limit, updated_at=now()`,
			userID, kycTier, int64(100_000_000_000), int64(1_000_000_000_000), 100)
		if err != nil {
			return err
		}
	}
	return nil
}

func newLedgerClient(addr string) (sarvexv1.LedgerClient, func(), error) {
	conn, err := grpc.NewClient(addr, grpc.WithTransportCredentials(insecure.NewCredentials()))
	if err != nil {
		return nil, nil, err
	}
	return sarvexv1.NewLedgerClient(conn), func() { _ = conn.Close() }, nil
}

func seedInitialBook(ctx context.Context, cfg config, bots []bot, rng *rand.Rand) {
	for level := int64(0); level < 7; level++ {
		qty := int64(30 + rng.Intn(80))
		buyPrice := 35 + level
		sellPrice := 65 - level
		_ = placeOrder(ctx, cfg, bots[int(level)%len(bots)], fmt.Sprintf("seed-bid-%d", level), "BUY", buyPrice, qty)
		_ = placeOrder(ctx, cfg, bots[(int(level)+len(bots)/2)%len(bots)], fmt.Sprintf("seed-ask-%d", level), "SELL", sellPrice, qty)
	}
}

func runRound(ctx context.Context, cfg config, bots []bot, rng *rand.Rand, round int) {
	fair := int64(47 + rng.Intn(9))
	makerCount := 4 + rng.Intn(5)
	for i := 0; i < makerCount; i++ {
		user := bots[rng.Intn(len(bots))]
		if rng.Intn(2) == 0 {
			price := clampTicks(fair - int64(8+rng.Intn(8)))
			qty := int64(5 + rng.Intn(45))
			_ = placeOrder(ctx, cfg, user, fmt.Sprintf("r%d-bid-%d", round, i), "BUY", price, qty)
		} else {
			price := clampTicks(fair + int64(8+rng.Intn(8)))
			qty := int64(5 + rng.Intn(45))
			_ = placeOrder(ctx, cfg, user, fmt.Sprintf("r%d-ask-%d", round, i), "SELL", price, qty)
		}
	}

	tradeCount := 1 + rng.Intn(3)
	for i := 0; i < tradeCount; i++ {
		user := bots[rng.Intn(len(bots))]
		qty := int64(3 + rng.Intn(20))
		if rng.Intn(2) == 0 {
			_ = placeOrder(ctx, cfg, user, fmt.Sprintf("r%d-take-buy-%d", round, i), "BUY", clampTicks(fair+25), qty)
		} else {
			_ = placeOrder(ctx, cfg, user, fmt.Sprintf("r%d-take-sell-%d", round, i), "SELL", clampTicks(fair-25), qty)
		}
	}
	log.Printf("round=%d fair=%d makers=%d takers=%d", round, fair, makerCount, tradeCount)
}

func placeOrder(ctx context.Context, cfg config, b bot, suffix, action string, price, qty int64) error {
	clientOrderID := fmt.Sprintf("sim-%s-%d-%s", b.userID, time.Now().UnixNano(), suffix)
	payload := map[string]any{
		"client_order_id": clientOrderID,
		"ticker":          cfg.ticker,
		"side":            "YES",
		"action":          action,
		"price_ticks":     price,
		"count":           qty,
		"tif":             "GTC",
	}
	var out map[string]any
	if err := postJSON(ctx, cfg.restURL+"/v1/orders", b.token, clientOrderID, payload, &out); err != nil {
		log.Printf("order rejected user=%s action=%s price=%d qty=%d err=%v", b.userID, action, price, qty, err)
		return err
	}
	return nil
}

func login(restURL, userID string) (string, error) {
	var out loginResp
	if err := postJSON(context.Background(), restURL+"/v1/auth/login", "", "", map[string]any{"user_id": userID}, &out); err != nil {
		return "", err
	}
	if out.Token == "" {
		return "", errors.New("login response missing token")
	}
	return out.Token, nil
}

func postJSON(ctx context.Context, url, token, idem string, payload any, out any) error {
	b, err := json.Marshal(payload)
	if err != nil {
		return err
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodPost, url, bytes.NewReader(b))
	if err != nil {
		return err
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
		return err
	}
	defer resp.Body.Close()
	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		var er errorResp
		_ = json.NewDecoder(resp.Body).Decode(&er)
		return fmt.Errorf("status=%d body=%v", resp.StatusCode, er.Error)
	}
	return json.NewDecoder(resp.Body).Decode(out)
}

func clampTicks(v int64) int64 {
	if v < 1 {
		return 1
	}
	if v > 99 {
		return 99
	}
	return v
}

func env(k, def string) string {
	if v := os.Getenv(k); v != "" {
		return v
	}
	return def
}
