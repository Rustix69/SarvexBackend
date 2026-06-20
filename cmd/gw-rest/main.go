package main

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"errors"
	"log"
	"net"
	"net/http"
	"os"
	"os/signal"
	"strconv"
	"strings"
	"sync"
	"syscall"
	"time"

	sarvexv1 "github.com/sarvex/proto/gen/go/sarvex/v1"
	"google.golang.org/grpc"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/credentials/insecure"
	"google.golang.org/grpc/status"
)

type errResp struct {
	Error map[string]any `json:"error"`
}

type gateway struct {
	orderClient    sarvexv1.OrderRouterClient
	ledgerClient   sarvexv1.LedgerClient
	matchClient    sarvexv1.MatchingEngineClient
	positionClient sarvexv1.PositionClient
	refClient      sarvexv1.RefDataClient
	mu             sync.Mutex
	idem           map[string]cachedResp
	simSeqs        map[string]simSeqState
}

type cachedResp struct {
	status int
	body   []byte
	ct     string
}

type simSeqState struct {
	seq      uint64
	advanced time.Time
}

type ctxKey string

const ctxUserIDKey ctxKey = "user_id"

func main() {
	ctx, stop := signal.NotifyContext(context.Background(), syscall.SIGINT, syscall.SIGTERM)
	defer stop()

	serviceName := getenv("SERVICE_NAME", "gw-rest")
	addr := ":" + getenv("HTTP_PORT", "8081")

	orderConn := mustDial(getenv("ORDER_ROUTER_ADDR", "order-router:50051"))
	defer orderConn.Close()
	ledgerConn := mustDial(getenv("LEDGER_ADDR", "ledger-svc:50051"))
	defer ledgerConn.Close()
	matchConn := mustDial(getenv("MATCHING_ADDR", "me-core:50051"))
	defer matchConn.Close()
	positionConn := mustDial(getenv("POSITION_ADDR", "position-svc:50051"))
	defer positionConn.Close()
	refConn := mustDial(getenv("REFDATA_ADDR", "refdata-svc:50051"))
	defer refConn.Close()

	gw := &gateway{
		orderClient:    sarvexv1.NewOrderRouterClient(orderConn),
		ledgerClient:   sarvexv1.NewLedgerClient(ledgerConn),
		matchClient:    sarvexv1.NewMatchingEngineClient(matchConn),
		positionClient: sarvexv1.NewPositionClient(positionConn),
		refClient:      sarvexv1.NewRefDataClient(refConn),
		idem:           map[string]cachedResp{},
		simSeqs:        map[string]simSeqState{},
	}

	mux := http.NewServeMux()
	mux.HandleFunc("/healthz", func(w http.ResponseWriter, _ *http.Request) {
		w.WriteHeader(http.StatusOK)
		_, _ = w.Write([]byte("ok"))
	})
	mux.HandleFunc("/readyz", func(w http.ResponseWriter, _ *http.Request) {
		w.WriteHeader(http.StatusOK)
		_, _ = w.Write([]byte("ready"))
	})
	mux.HandleFunc("/v1/auth/login", gw.login)
	mux.HandleFunc("/v1/orders", gw.withAuth(gw.orders))
	mux.HandleFunc("/v1/orders/", gw.withAuth(gw.orderByID))
	mux.HandleFunc("/v1/markets", gw.marketsRoot)
	mux.HandleFunc("/v1/markets/", gw.markets)
	mux.HandleFunc("/v1/account/balance", gw.withAuth(gw.balance))
	mux.HandleFunc("/v1/account/history", gw.withAuth(gw.history))
	mux.HandleFunc("/v1/positions", gw.withAuth(gw.positions))
	mux.HandleFunc("/v1/health/overview", gw.healthOverview)
	mux.HandleFunc("/v1/demo/deposits/credit", gw.withAuth(gw.demoDeposit))
	mux.HandleFunc("/v1/admin/deposits/credit", gw.withAuth(gw.adminDeposit))

	srv := &http.Server{Addr: addr, Handler: withCORS(mux), ReadHeaderTimeout: 3 * time.Second}
	go func() {
		<-ctx.Done()
		shutdownCtx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cancel()
		_ = srv.Shutdown(shutdownCtx)
	}()

	log.Printf("service=%s msg=starting addr=%s", serviceName, addr)
	if err := srv.ListenAndServe(); err != nil && !errors.Is(err, http.ErrServerClosed) {
		log.Fatal(err)
	}
}

func (g *gateway) login(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		return
	}
	var in struct {
		UserID string `json:"user_id"`
	}
	if err := json.NewDecoder(r.Body).Decode(&in); err != nil || strings.TrimSpace(in.UserID) == "" {
		writeErr(w, http.StatusBadRequest, "INVALID_ARGUMENT", "user_id is required")
		return
	}
	tok := "demo." + base64.RawURLEncoding.EncodeToString([]byte(strings.TrimSpace(in.UserID)))
	writeJSON(w, http.StatusOK, map[string]any{"token": tok, "token_type": "Bearer"})
}

func (g *gateway) withAuth(next http.HandlerFunc) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		if r.Method == http.MethodOptions {
			w.WriteHeader(http.StatusNoContent)
			return
		}
		h := r.Header.Get("Authorization")
		parts := strings.SplitN(h, " ", 2)
		if len(parts) != 2 || !strings.EqualFold(parts[0], "Bearer") {
			writeErr(w, http.StatusUnauthorized, "UNAUTHENTICATED", "bearer token required")
			return
		}
		userID, ok := parseDemoToken(parts[1])
		if !ok {
			writeErr(w, http.StatusUnauthorized, "UNAUTHENTICATED", "invalid token")
			return
		}
		next(w, r.WithContext(context.WithValue(r.Context(), ctxUserIDKey, userID)))
	}
}

func withCORS(next http.Handler) http.Handler {
	allowedOrigins := parseAllowedOrigins(getenv("ALLOWED_ORIGINS", "*"))
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		origin := r.Header.Get("Origin")
		if origin != "" && originAllowed(origin, allowedOrigins) {
			w.Header().Set("Access-Control-Allow-Origin", origin)
			w.Header().Set("Vary", "Origin")
		}
		w.Header().Set("Access-Control-Allow-Methods", "GET,POST,OPTIONS")
		w.Header().Set("Access-Control-Allow-Headers", "Authorization,Content-Type,Idempotency-Key")
		if r.Method == http.MethodOptions {
			w.WriteHeader(http.StatusNoContent)
			return
		}
		next.ServeHTTP(w, r)
	})
}

func parseAllowedOrigins(raw string) map[string]bool {
	out := map[string]bool{}
	for _, part := range strings.Split(raw, ",") {
		part = strings.TrimSpace(part)
		if part != "" {
			out[part] = true
		}
	}
	if len(out) == 0 {
		out["*"] = true
	}
	return out
}

func originAllowed(origin string, allowed map[string]bool) bool {
	return allowed["*"] || allowed[origin]
}

func (g *gateway) orders(w http.ResponseWriter, r *http.Request) {
	userID := r.Context().Value(ctxUserIDKey).(string)
	switch r.Method {
	case http.MethodPost:
		if !g.requireIdem(w, r, userID) {
			return
		}
		var in struct {
			ClientOrderID string `json:"client_order_id"`
			Ticker        string `json:"ticker"`
			Side          string `json:"side"`
			Action        string `json:"action"`
			OrderType     string `json:"order_type"`
			Type          string `json:"type"`
			PriceTicks    int64  `json:"price_ticks"`
			Count         int64  `json:"count"`
			TIF           string `json:"tif"`
			PostOnly      bool   `json:"post_only"`
			ReduceOnly    bool   `json:"reduce_only"`
		}
		if err := json.NewDecoder(r.Body).Decode(&in); err != nil {
			writeErr(w, http.StatusBadRequest, "INVALID_ARGUMENT", "invalid json")
			return
		}
		orderType := strings.ToUpper(strings.TrimSpace(in.OrderType))
		if orderType == "" {
			orderType = strings.ToUpper(strings.TrimSpace(in.Type))
		}
		if orderType == "MARKET" {
			in.PriceTicks = 0
			in.TIF = "IOC"
			in.PostOnly = false
		}
		ctx, cancel := context.WithTimeout(r.Context(), 2*time.Second)
		defer cancel()
		resp, err := g.orderClient.SubmitOrder(ctx, &sarvexv1.SubmitOrderRequest{
			UserId:        userID,
			ClientOrderId: in.ClientOrderID,
			Ticker:        in.Ticker,
			Side:          sideFromString(in.Side),
			Action:        actionFromString(in.Action),
			PriceTicks:    in.PriceTicks,
			Count:         in.Count,
			Tif:           tifFromString(in.TIF),
			PostOnly:      in.PostOnly,
			ReduceOnly:    in.ReduceOnly,
		})
		if err != nil {
			writeGrpcErr(w, err)
			return
		}
		g.writeIdemCapture(w, r, userID, http.StatusOK, resp)
	case http.MethodGet:
		limit, _ := strconv.Atoi(r.URL.Query().Get("limit"))
		ctx, cancel := context.WithTimeout(r.Context(), 2*time.Second)
		defer cancel()
		resp, err := g.orderClient.ListOrders(ctx, &sarvexv1.ListOrdersRequest{
			UserId: userID, Ticker: r.URL.Query().Get("ticker"), Limit: int32(limit), Cursor: r.URL.Query().Get("cursor"),
		})
		if err != nil {
			writeGrpcErr(w, err)
			return
		}
		writeJSON(w, http.StatusOK, resp)
	default:
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
	}
}

func (g *gateway) orderByID(w http.ResponseWriter, r *http.Request) {
	userID := r.Context().Value(ctxUserIDKey).(string)
	parts := strings.Split(strings.TrimPrefix(r.URL.Path, "/v1/orders/"), "/")
	if len(parts) == 0 || strings.TrimSpace(parts[0]) == "" {
		writeErr(w, http.StatusBadRequest, "INVALID_ARGUMENT", "order id required")
		return
	}
	id := parts[0]
	if len(parts) == 1 && r.Method == http.MethodGet {
		ctx, cancel := context.WithTimeout(r.Context(), 2*time.Second)
		defer cancel()
		resp, err := g.orderClient.GetOrder(ctx, &sarvexv1.GetOrderRequest{UserId: userID, Key: &sarvexv1.GetOrderRequest_OrderId{OrderId: id}})
		if err != nil {
			writeGrpcErr(w, err)
			return
		}
		writeJSON(w, http.StatusOK, resp)
		return
	}
	if len(parts) == 2 && parts[1] == "cancel" && r.Method == http.MethodPost {
		if !g.requireIdem(w, r, userID) {
			return
		}
		ctx, cancel := context.WithTimeout(r.Context(), 2*time.Second)
		defer cancel()
		resp, err := g.orderClient.CancelOrder(ctx, &sarvexv1.CancelOrderRequest{UserId: userID, OrderId: id})
		if err != nil {
			writeGrpcErr(w, err)
			return
		}
		g.writeIdemCapture(w, r, userID, http.StatusOK, resp)
		return
	}
	http.Error(w, "not found", http.StatusNotFound)
}

func (g *gateway) marketsRoot(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodGet {
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		return
	}
	limit, _ := strconv.Atoi(r.URL.Query().Get("limit"))
	ctx, cancel := context.WithTimeout(r.Context(), 2*time.Second)
	defer cancel()
	resp, err := g.refClient.ListContracts(ctx, &sarvexv1.ListContractsRequest{
		State:        contractStateFromString(r.URL.Query().Get("state")),
		SeriesTicker: r.URL.Query().Get("series_ticker"),
		Limit:        int32(limit),
		Cursor:       r.URL.Query().Get("cursor"),
	})
	if err != nil {
		writeGrpcErr(w, err)
		return
	}
	if r.URL.Query().Get("include_test") != "true" {
		resp.Contracts = publicContracts(resp.GetContracts())
		resp.NextCursor = ""
	}
	writeJSON(w, http.StatusOK, resp)
}

func (g *gateway) markets(w http.ResponseWriter, r *http.Request) {
	p := strings.TrimPrefix(r.URL.Path, "/v1/markets/")
	parts := strings.Split(p, "/")
	if len(parts) == 0 || parts[0] == "" {
		writeErr(w, http.StatusBadRequest, "INVALID_ARGUMENT", "ticker required")
		return
	}
	ticker := parts[0]
	ctx, cancel := context.WithTimeout(r.Context(), 2*time.Second)
	defer cancel()
	if len(parts) == 1 {
		resp, err := g.refClient.GetContract(ctx, &sarvexv1.GetContractRequest{Ticker: ticker})
		if err != nil {
			writeGrpcErr(w, err)
			return
		}
		writeJSON(w, http.StatusOK, resp)
		return
	}
	if len(parts) == 2 && parts[1] == "orderbook" {
		depth, _ := strconv.Atoi(r.URL.Query().Get("depth"))
		resp, err := g.matchClient.GetBookSnapshot(ctx, &sarvexv1.GetBookSnapshotRequest{Ticker: ticker, Depth: int32(depth)})
		if err != nil {
			writeGrpcErr(w, err)
			return
		}
		writeJSON(w, http.StatusOK, resp)
		return
	}
	if len(parts) == 2 && parts[1] == "fills" {
		from, _ := strconv.ParseUint(r.URL.Query().Get("from_global_seq"), 10, 64)
		to, _ := strconv.ParseUint(r.URL.Query().Get("to_global_seq"), 10, 64)
		limit, _ := strconv.Atoi(r.URL.Query().Get("limit"))
		resp, err := g.orderClient.ListFills(ctx, &sarvexv1.ListFillsRequest{Ticker: ticker, FromGlobalSeq: from, ToGlobalSeq: to, Limit: int32(limit), Cursor: r.URL.Query().Get("cursor")})
		if err != nil {
			writeGrpcErr(w, err)
			return
		}
		writeJSON(w, http.StatusOK, resp)
		return
	}
	http.Error(w, "not found", http.StatusNotFound)
}

func (g *gateway) balance(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodGet {
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		return
	}
	userID := r.Context().Value(ctxUserIDKey).(string)
	ctx, cancel := context.WithTimeout(r.Context(), 2*time.Second)
	defer cancel()
	resp, err := g.ledgerClient.GetBalance(ctx, &sarvexv1.GetBalanceRequest{UserId: userID})
	if err != nil {
		writeGrpcErr(w, err)
		return
	}
	writeJSON(w, http.StatusOK, resp)
}

func (g *gateway) history(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodGet {
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		return
	}
	userID := r.Context().Value(ctxUserIDKey).(string)
	limit, _ := strconv.Atoi(r.URL.Query().Get("limit"))
	ctx, cancel := context.WithTimeout(r.Context(), 2*time.Second)
	defer cancel()
	resp, err := g.ledgerClient.GetAccountHistory(ctx, &sarvexv1.GetAccountHistoryRequest{UserId: userID, Limit: int32(limit), Cursor: r.URL.Query().Get("cursor")})
	if err != nil {
		writeGrpcErr(w, err)
		return
	}
	writeJSON(w, http.StatusOK, resp)
}

func (g *gateway) positions(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodGet {
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		return
	}
	userID := r.Context().Value(ctxUserIDKey).(string)
	ctx, cancel := context.WithTimeout(r.Context(), 2*time.Second)
	defer cancel()
	resp, err := g.positionClient.ListPositions(ctx, &sarvexv1.ListPositionsRequest{UserId: userID, IncludeClosed: r.URL.Query().Get("include_closed") == "true"})
	if err != nil {
		writeGrpcErr(w, err)
		return
	}
	writeJSON(w, http.StatusOK, resp)
}

type healthItem struct {
	Name      string `json:"name"`
	Kind      string `json:"kind"`
	Status    string `json:"status"`
	Message   string `json:"message"`
	Target    string `json:"target,omitempty"`
	LatencyMS int64  `json:"latency_ms"`
	CheckedAt string `json:"checked_at"`
}

func (g *gateway) healthOverview(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodGet {
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		return
	}
	ctx, cancel := context.WithTimeout(r.Context(), 3*time.Second)
	defer cancel()

	items := []healthItem{
		{Name: "gw-rest", Kind: "backend", Status: "running", Message: "ready", Target: "self", CheckedAt: time.Now().UTC().Format(time.RFC3339Nano)},
	}
	for _, infra := range infrastructureHealthChecks() {
		items = append(items, checkTCPReady(ctx, infra.name, infra.target))
	}
	for _, svc := range backendHealthChecks() {
		items = append(items, checkHTTPReady(ctx, svc.name, svc.kind, svc.target))
	}
	items = append(items, g.checkSimulator(ctx, "Binary market simulator", []string{"DEMO-AI-DEC26-1T", "DEMO-BTC-JUN26-120K", "DEMO-ETH-JUN26-8K", "DEMO-FED-JUL26-CUT", "DEMO-NVIDIA-AUG26-5T", "DEMO-OIL-MAY26-95", "DEMO-TESLA-Q2-26-500K", "DEMO-US-HOUSE-2026-DEM", "DEMO-WC-2026-FRANCE"}))
	items = append(items, g.checkSimulator(ctx, "Futures simulator", []string{"INDIA-CPI-JUN26-SCALAR", "FUT-INDIA-GDP-FY26-SCALAR", "FUT-BTC-JUN26-LEVEL", "FUT-ETH-JUN26-LEVEL", "FUT-AI-MCAP-DEC26-SCALAR", "FUT-INDIA-UNEMP-DEC26-SCALAR", "FUT-USDINR-DEC26-SCALAR", "FUT-NIFTY-DEC26-LEVEL", "FUT-FEDRATE-DEC26-SCALAR"}))

	running := 0
	for _, item := range items {
		if item.Status == "running" {
			running++
		}
	}
	writeJSON(w, http.StatusOK, map[string]any{
		"generated_at": time.Now().UTC().Format(time.RFC3339Nano),
		"summary":      map[string]any{"running": running, "total": len(items), "not_running": len(items) - running},
		"items":        items,
	})
}

func infrastructureHealthChecks() []struct {
	name   string
	target string
} {
	return []struct {
		name   string
		target string
	}{
		{"postgres", "postgres:5432"},
		{"redis", "redis:6379"},
		{"nats", "nats:4222"},
	}
}

func backendHealthChecks() []struct {
	name   string
	kind   string
	target string
} {
	return []struct {
		name   string
		kind   string
		target string
	}{
		{"refdata-svc", "backend", "http://refdata-svc:8080/readyz"},
		{"ledger-svc", "backend", "http://ledger-svc:8080/readyz"},
		{"risk-svc", "backend", "http://risk-svc:8080/readyz"},
		{"me-core", "backend", "http://me-core:8080/readyz"},
		{"order-router", "backend", "http://order-router:8080/readyz"},
		{"position-svc", "backend", "http://position-svc:8080/readyz"},
		{"oracle-svc", "backend", "http://oracle-svc:8080/readyz"},
		{"settlement-svc", "backend", "http://settlement-svc:8080/readyz"},
		{"audit-svc", "backend", "http://audit-svc:8080/readyz"},
		{"admin-svc", "backend", "http://admin-svc:8080/readyz"},
		{"gw-ws", "backend", "http://gw-ws:8082/readyz"},
	}
}

func checkTCPReady(ctx context.Context, name, target string) healthItem {
	start := time.Now()
	item := healthItem{Name: name, Kind: "infrastructure", Target: target, CheckedAt: start.UTC().Format(time.RFC3339Nano)}
	dialer := net.Dialer{Timeout: 700 * time.Millisecond}
	conn, err := dialer.DialContext(ctx, "tcp", target)
	item.LatencyMS = time.Since(start).Milliseconds()
	if err != nil {
		item.Status = "not_running"
		item.Message = err.Error()
		return item
	}
	_ = conn.Close()
	item.Status = "running"
	item.Message = "tcp reachable"
	return item
}

func checkHTTPReady(ctx context.Context, name, kind, target string) healthItem {
	start := time.Now()
	item := healthItem{Name: name, Kind: kind, Target: target, CheckedAt: start.UTC().Format(time.RFC3339Nano)}
	reqCtx, cancel := context.WithTimeout(ctx, 800*time.Millisecond)
	defer cancel()
	req, err := http.NewRequestWithContext(reqCtx, http.MethodGet, target, nil)
	if err != nil {
		item.Status = "not_running"
		item.Message = err.Error()
		return item
	}
	resp, err := http.DefaultClient.Do(req)
	item.LatencyMS = time.Since(start).Milliseconds()
	if err != nil {
		item.Status = "not_running"
		item.Message = err.Error()
		return item
	}
	defer resp.Body.Close()
	if resp.StatusCode >= 200 && resp.StatusCode < 300 {
		item.Status = "running"
		item.Message = "ready"
		return item
	}
	item.Status = "not_running"
	item.Message = resp.Status
	return item
}

func (g *gateway) checkSimulator(ctx context.Context, name string, tickers []string) healthItem {
	start := time.Now()
	runningBooks := 0
	freshBooks := 0
	checked := 0
	var failures []string
	for _, ticker := range tickers {
		reqCtx, cancel := context.WithTimeout(ctx, 600*time.Millisecond)
		book, err := g.matchClient.GetBookSnapshot(reqCtx, &sarvexv1.GetBookSnapshotRequest{Ticker: ticker, Depth: 1})
		cancel()
		checked++
		if err != nil {
			failures = append(failures, ticker+": "+err.Error())
			continue
		}
		if len(book.GetBids()) > 0 && len(book.GetAsks()) > 0 && book.GetSeq() > 0 {
			runningBooks++
			if g.noteSimulatorSeq(name, ticker, book.GetSeq(), start) {
				freshBooks++
			}
			continue
		}
		failures = append(failures, ticker+": missing bid/ask")
	}

	item := healthItem{
		Name:      name,
		Kind:      "simulator",
		Target:    strings.Join(tickers, ", "),
		LatencyMS: time.Since(start).Milliseconds(),
		CheckedAt: start.UTC().Format(time.RFC3339Nano),
	}
	if runningBooks >= minHealthyBooks(len(tickers)) && freshBooks > 0 {
		item.Status = "running"
		item.Message = "fresh live books " + strconv.Itoa(freshBooks) + "/" + strconv.Itoa(checked)
		return item
	}
	item.Status = "not_running"
	item.Message = "fresh live books " + strconv.Itoa(freshBooks) + "/" + strconv.Itoa(checked) + ", bid/ask books " + strconv.Itoa(runningBooks) + "/" + strconv.Itoa(checked)
	if len(failures) > 0 {
		item.Message += ": " + strings.Join(failures, "; ")
	}
	return item
}

func (g *gateway) noteSimulatorSeq(simName, ticker string, seq uint64, now time.Time) bool {
	key := simName + "|" + ticker
	g.mu.Lock()
	defer g.mu.Unlock()
	state := g.simSeqs[key]
	if state.seq == 0 || seq > state.seq {
		state.seq = seq
		state.advanced = now
		g.simSeqs[key] = state
		return true
	}
	if !state.advanced.IsZero() && now.Sub(state.advanced) <= 12*time.Second {
		return true
	}
	return false
}

func minHealthyBooks(total int) int {
	if total <= 2 {
		return total
	}
	return 2
}

func (g *gateway) demoDeposit(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		return
	}
	userID := r.Context().Value(ctxUserIDKey).(string)
	var in struct {
		AmountMicroUSDC int64  `json:"amount_micro_usdc"`
		AmountUSDC      int64  `json:"amount_usdc"`
		Note            string `json:"note"`
	}
	if err := json.NewDecoder(r.Body).Decode(&in); err != nil {
		writeErr(w, http.StatusBadRequest, "INVALID_ARGUMENT", "invalid json")
		return
	}
	amount := in.AmountMicroUSDC
	if amount == 0 && in.AmountUSDC > 0 {
		amount = in.AmountUSDC * 1_000_000
	}
	if amount <= 0 {
		writeErr(w, http.StatusBadRequest, "INVALID_ARGUMENT", "amount is required")
		return
	}
	if amount > 1_000_000_000_000 {
		writeErr(w, http.StatusBadRequest, "INVALID_ARGUMENT", "demo deposit limit exceeded")
		return
	}
	ctx, cancel := context.WithTimeout(r.Context(), 6*time.Second)
	defer cancel()
	if _, err := g.ledgerClient.AdminCreditDeposit(ctx, &sarvexv1.AdminCreditDepositRequest{
		UserId:          userID,
		AmountMicroUsdc: amount,
		Note:            defaultString(in.Note, "frontend demo deposit"),
	}); err != nil {
		writeGrpcErr(w, err)
		return
	}
	balanceCtx, balanceCancel := context.WithTimeout(r.Context(), 3*time.Second)
	defer balanceCancel()
	resp, err := g.ledgerClient.GetBalance(balanceCtx, &sarvexv1.GetBalanceRequest{UserId: userID})
	if err != nil {
		writeGrpcErr(w, err)
		return
	}
	writeJSON(w, http.StatusOK, map[string]any{"ok": true, "balance": resp})
}

func (g *gateway) adminDeposit(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		return
	}
	adminID := r.Context().Value(ctxUserIDKey).(string)
	if adminID != "u_admin" && adminID != "u_admin_1" {
		writeErr(w, http.StatusForbidden, "PERMISSION_DENIED", "admin user required")
		return
	}
	var in struct {
		UserID          string `json:"user_id"`
		AmountMicroUSDC int64  `json:"amount_micro_usdc"`
		AmountUSDC      int64  `json:"amount_usdc"`
		Note            string `json:"note"`
	}
	if err := json.NewDecoder(r.Body).Decode(&in); err != nil || strings.TrimSpace(in.UserID) == "" {
		writeErr(w, http.StatusBadRequest, "INVALID_ARGUMENT", "user_id is required")
		return
	}
	amount := in.AmountMicroUSDC
	if amount == 0 && in.AmountUSDC > 0 {
		amount = in.AmountUSDC * 1_000_000
	}
	if amount <= 0 {
		writeErr(w, http.StatusBadRequest, "INVALID_ARGUMENT", "amount is required")
		return
	}
	ctx, cancel := context.WithTimeout(r.Context(), 2*time.Second)
	defer cancel()
	if _, err := g.ledgerClient.AdminCreditDeposit(ctx, &sarvexv1.AdminCreditDepositRequest{
		UserId:          strings.TrimSpace(in.UserID),
		AmountMicroUsdc: amount,
		Note:            defaultString(in.Note, "frontend admin deposit"),
	}); err != nil {
		writeGrpcErr(w, err)
		return
	}
	writeJSON(w, http.StatusOK, map[string]any{"ok": true})
}

func (g *gateway) requireIdem(w http.ResponseWriter, r *http.Request, userID string) bool {
	key := strings.TrimSpace(r.Header.Get("Idempotency-Key"))
	if key == "" {
		writeErr(w, http.StatusBadRequest, "IDEMPOTENCY_KEY_REQUIRED", "Idempotency-Key header is required")
		return false
	}
	idemKey := userID + "|" + r.URL.Path + "|" + key
	g.mu.Lock()
	cached, ok := g.idem[idemKey]
	g.mu.Unlock()
	if ok {
		if cached.ct != "" {
			w.Header().Set("Content-Type", cached.ct)
		}
		w.WriteHeader(cached.status)
		_, _ = w.Write(cached.body)
		return false
	}
	return true
}

func (g *gateway) writeIdemCapture(w http.ResponseWriter, r *http.Request, userID string, code int, payload any) {
	b, _ := json.Marshal(payload)
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(code)
	_, _ = w.Write(b)
	key := strings.TrimSpace(r.Header.Get("Idempotency-Key"))
	if key != "" {
		g.mu.Lock()
		g.idem[userID+"|"+r.URL.Path+"|"+key] = cachedResp{status: code, body: b, ct: "application/json"}
		g.mu.Unlock()
	}
}

func defaultString(v, def string) string {
	if strings.TrimSpace(v) == "" {
		return def
	}
	return strings.TrimSpace(v)
}

func mustDial(addr string) *grpc.ClientConn {
	conn, err := grpc.NewClient(addr, grpc.WithTransportCredentials(insecure.NewCredentials()))
	if err != nil {
		log.Fatal(err)
	}
	return conn
}

func parseDemoToken(tok string) (string, bool) {
	if !strings.HasPrefix(tok, "demo.") {
		return "", false
	}
	raw := strings.TrimPrefix(tok, "demo.")
	b, err := base64.RawURLEncoding.DecodeString(raw)
	if err != nil || strings.TrimSpace(string(b)) == "" {
		return "", false
	}
	return string(b), true
}

func writeJSON(w http.ResponseWriter, code int, v any) {
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(code)
	_ = json.NewEncoder(w).Encode(v)
}

func writeErr(w http.ResponseWriter, code int, c, m string) {
	writeJSON(w, code, errResp{Error: map[string]any{"code": c, "message": m}})
}

func writeGrpcErr(w http.ResponseWriter, err error) {
	st, ok := status.FromError(err)
	if !ok {
		writeErr(w, http.StatusInternalServerError, "INTERNAL", err.Error())
		return
	}
	hc := http.StatusInternalServerError
	switch st.Code() {
	case codes.InvalidArgument:
		hc = http.StatusBadRequest
	case codes.NotFound:
		hc = http.StatusNotFound
	case codes.AlreadyExists:
		hc = http.StatusConflict
	case codes.Unauthenticated:
		hc = http.StatusUnauthorized
	case codes.PermissionDenied:
		hc = http.StatusForbidden
	case codes.FailedPrecondition:
		hc = http.StatusPreconditionFailed
	case codes.ResourceExhausted:
		hc = http.StatusTooManyRequests
	case codes.DeadlineExceeded:
		hc = http.StatusGatewayTimeout
	}
	writeErr(w, hc, st.Code().String(), st.Message())
}

func sideFromString(v string) sarvexv1.Side {
	switch strings.ToUpper(strings.TrimSpace(v)) {
	case "NO":
		return sarvexv1.Side_SIDE_NO
	case "LONG":
		return sarvexv1.Side_SIDE_LONG
	case "SHORT":
		return sarvexv1.Side_SIDE_SHORT
	default:
		return sarvexv1.Side_SIDE_YES
	}
}
func actionFromString(v string) sarvexv1.Action {
	if strings.EqualFold(strings.TrimSpace(v), "SELL") {
		return sarvexv1.Action_ACTION_SELL
	}
	return sarvexv1.Action_ACTION_BUY
}
func tifFromString(v string) sarvexv1.TimeInForce {
	switch strings.ToUpper(strings.TrimSpace(v)) {
	case "IOC":
		return sarvexv1.TimeInForce_TIME_IN_FORCE_IOC
	case "FOK":
		return sarvexv1.TimeInForce_TIME_IN_FORCE_FOK
	default:
		return sarvexv1.TimeInForce_TIME_IN_FORCE_GTC
	}
}

func contractStateFromString(v string) sarvexv1.ContractState {
	switch strings.ToUpper(strings.TrimSpace(v)) {
	case "OPEN":
		return sarvexv1.ContractState_CONTRACT_STATE_OPEN
	case "CLOSED":
		return sarvexv1.ContractState_CONTRACT_STATE_CLOSED
	case "SETTLED":
		return sarvexv1.ContractState_CONTRACT_STATE_SETTLED
	case "HALTED":
		return sarvexv1.ContractState_CONTRACT_STATE_HALTED
	case "CANCELLED":
		return sarvexv1.ContractState_CONTRACT_STATE_CANCELLED
	default:
		return sarvexv1.ContractState_CONTRACT_STATE_UNSPECIFIED
	}
}

func publicContracts(in []*sarvexv1.Contract) []*sarvexv1.Contract {
	out := make([]*sarvexv1.Contract, 0, len(in))
	for _, c := range in {
		if strings.HasPrefix(c.GetTicker(), "TEST-") || strings.HasPrefix(c.GetEventTicker(), "TEST-") || strings.HasPrefix(c.GetSeriesTicker(), "TEST-") {
			continue
		}
		out = append(out, c)
	}
	return out
}

func getenv(k, def string) string {
	if v := os.Getenv(k); v != "" {
		return v
	}
	return def
}
