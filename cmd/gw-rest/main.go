package main

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"errors"
	"log"
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
	positionClient sarvexv1.PositionClient
	refClient      sarvexv1.RefDataClient
	mu             sync.Mutex
	idem           map[string]cachedResp
}

type cachedResp struct {
	status int
	body   []byte
	ct     string
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
	positionConn := mustDial(getenv("POSITION_ADDR", "position-svc:50051"))
	defer positionConn.Close()
	refConn := mustDial(getenv("REFDATA_ADDR", "refdata-svc:50051"))
	defer refConn.Close()

	gw := &gateway{
		orderClient:    sarvexv1.NewOrderRouterClient(orderConn),
		ledgerClient:   sarvexv1.NewLedgerClient(ledgerConn),
		positionClient: sarvexv1.NewPositionClient(positionConn),
		refClient:      sarvexv1.NewRefDataClient(refConn),
		idem:           map[string]cachedResp{},
	}

	mux := http.NewServeMux()
	mux.HandleFunc("/healthz", func(w http.ResponseWriter, _ *http.Request) { w.WriteHeader(http.StatusOK); _, _ = w.Write([]byte("ok")) })
	mux.HandleFunc("/readyz", func(w http.ResponseWriter, _ *http.Request) { w.WriteHeader(http.StatusOK); _, _ = w.Write([]byte("ready")) })
	mux.HandleFunc("/v1/auth/login", gw.login)
	mux.HandleFunc("/v1/orders", gw.withAuth(gw.orders))
	mux.HandleFunc("/v1/orders/", gw.withAuth(gw.orderByID))
	mux.HandleFunc("/v1/markets/", gw.markets)
	mux.HandleFunc("/v1/account/balance", gw.withAuth(gw.balance))
	mux.HandleFunc("/v1/account/history", gw.withAuth(gw.history))
	mux.HandleFunc("/v1/positions", gw.withAuth(gw.positions))

	srv := &http.Server{Addr: addr, Handler: mux, ReadHeaderTimeout: 3 * time.Second}
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
	var in struct{ UserID string `json:"user_id"` }
	if err := json.NewDecoder(r.Body).Decode(&in); err != nil || strings.TrimSpace(in.UserID) == "" {
		writeErr(w, http.StatusBadRequest, "INVALID_ARGUMENT", "user_id is required")
		return
	}
	tok := "demo." + base64.RawURLEncoding.EncodeToString([]byte(strings.TrimSpace(in.UserID)))
	writeJSON(w, http.StatusOK, map[string]any{"token": tok, "token_type": "Bearer"})
}

func (g *gateway) withAuth(next http.HandlerFunc) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
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

func getenv(k, def string) string {
	if v := os.Getenv(k); v != "" {
		return v
	}
	return def
}
