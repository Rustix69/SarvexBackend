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

	"github.com/jackc/pgx/v5/pgxpool"
	"github.com/nats-io/nats.go"
	sarvexv1 "github.com/sarvex/proto/gen/go/sarvex/v1"
	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials/insecure"
	"nhooyr.io/websocket"
)

type hub struct {
	nc         *nats.Conn
	pg         *pgxpool.Pool
	order      sarvexv1.OrderRouterClient
	service    string
	sendBufCap int
}

type wsConn struct {
	c        *websocket.Conn
	sendCh   chan []byte
	userID   string
	subs     []*nats.Subscription
	subsMu   sync.Mutex
	ctx      context.Context
	cancel   context.CancelFunc
	hub      *hub
}

func main() {
	ctx, stop := signal.NotifyContext(context.Background(), syscall.SIGINT, syscall.SIGTERM)
	defer stop()

	serviceName := getenv("SERVICE_NAME", "gw-ws")
	addr := ":" + getenv("HTTP_PORT", "8082")
	postgresURL := os.Getenv("POSTGRES_URL")
	natsURL := os.Getenv("NATS_URL")

	pg, err := pgxpool.New(ctx, postgresURL)
	if err != nil {
		log.Fatal(err)
	}
	defer pg.Close()
	if err := pg.Ping(ctx); err != nil {
		log.Fatal(err)
	}

	nc, err := nats.Connect(natsURL, nats.Name(serviceName))
	if err != nil {
		log.Fatal(err)
	}
	defer nc.Close()

	orderConn, err := grpc.NewClient(getenv("ORDER_ROUTER_ADDR", "order-router:50051"), grpc.WithTransportCredentials(insecure.NewCredentials()))
	if err != nil {
		log.Fatal(err)
	}
	defer orderConn.Close()

	h := &hub{nc: nc, pg: pg, order: sarvexv1.NewOrderRouterClient(orderConn), service: serviceName, sendBufCap: 256}

	mux := http.NewServeMux()
	mux.HandleFunc("/healthz", func(w http.ResponseWriter, _ *http.Request) {
		w.WriteHeader(http.StatusOK)
		_, _ = w.Write([]byte("ok"))
	})
	mux.HandleFunc("/readyz", func(w http.ResponseWriter, _ *http.Request) {
		if pg.Ping(context.Background()) != nil || nc.Status() != nats.CONNECTED {
			w.WriteHeader(http.StatusServiceUnavailable)
			_, _ = w.Write([]byte("not ready"))
			return
		}
		w.WriteHeader(http.StatusOK)
		_, _ = w.Write([]byte("ready"))
	})
	mux.HandleFunc("/ws", h.handleWS)

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

func (h *hub) handleWS(w http.ResponseWriter, r *http.Request) {
	c, err := websocket.Accept(w, r, nil)
	if err != nil {
		return
	}
	ctx, cancel := context.WithCancel(r.Context())
	wc := &wsConn{c: c, sendCh: make(chan []byte, h.sendBufCap), ctx: ctx, cancel: cancel, hub: h}
	defer wc.close(websocket.StatusNormalClosure, "bye")

	wc.send(map[string]any{"type": "welcome", "msg": map[string]any{"service": h.service, "ts": time.Now().UTC().Format(time.RFC3339Nano)}})
	go wc.writer()
	wc.reader()
}

func (wc *wsConn) reader() {
	for {
		_, b, err := wc.c.Read(wc.ctx)
		if err != nil {
			return
		}
		var in map[string]any
		if err := json.Unmarshal(b, &in); err != nil {
			wc.sendErr("INVALID_JSON", "could not parse message")
			continue
		}
		op, _ := in["op"].(string)
		switch op {
		case "auth":
			tok, _ := in["token"].(string)
			uid, ok := parseDemoToken(tok)
			if !ok {
				wc.sendErr("UNAUTHENTICATED", "invalid token")
				continue
			}
			wc.userID = uid
			wc.send(map[string]any{"type": "auth.ok", "user_id": uid})
		case "subscribe":
			ch, _ := in["channel"].(string)
			ticker, _ := in["ticker"].(string)
			wc.handleSubscribe(ch, ticker)
		default:
			wc.sendErr("INVALID_OP", "unknown op")
		}
	}
}

func (wc *wsConn) handleSubscribe(channel, ticker string) {
	switch channel {
	case "market":
		if strings.TrimSpace(ticker) == "" {
			wc.sendErr("INVALID_ARGUMENT", "ticker required")
			return
		}
		wc.subscribeMarket(ticker)
	case "private":
		if wc.userID == "" {
			wc.sendErr("UNAUTHENTICATED", "auth required")
			return
		}
		wc.subscribePrivate(wc.userID)
	default:
		wc.sendErr("INVALID_CHANNEL", "unsupported channel")
	}
}

func (wc *wsConn) subscribeMarket(ticker string) {
	buffer := make([][]byte, 0, 256)
	var bufMu sync.Mutex
	start := time.Now().UTC()

	sub, err := wc.hub.nc.Subscribe("md.trade."+ticker, func(msg *nats.Msg) {
		bufMu.Lock()
		if len(buffer) < 4096 {
			cp := append([]byte(nil), msg.Data...)
			buffer = append(buffer, cp)
		}
		bufMu.Unlock()
	})
	if err != nil {
		wc.sendErr("SUBSCRIBE_FAILED", err.Error())
		return
	}
	wc.addSub(sub)

	var snapshotSeq uint64
	_ = wc.hub.pg.QueryRow(wc.ctx, `SELECT COALESCE(MAX(global_seq),0) FROM orders.fills WHERE ticker=$1`, ticker).Scan(&snapshotSeq)
	wc.send(map[string]any{"type": "snapshot", "channel": "market", "ticker": ticker, "seq": snapshotSeq, "as_of": start.Format(time.RFC3339Nano)})

	bufMu.Lock()
	replay := append([][]byte(nil), buffer...)
	buffer = nil
	bufMu.Unlock()
	for _, m := range replay {
		if seqFromMsg(m) <= snapshotSeq {
			continue
		}
		wc.sendRaw(m)
	}

	wc.send(map[string]any{"type": "subscribed", "channel": "market", "ticker": ticker})
}

func (wc *wsConn) subscribePrivate(userID string) {
	subs := []string{"exec.user." + userID, "exec.fills.user." + userID, "ledger.balance.user." + userID}
	for _, subj := range subs {
		sub, err := wc.hub.nc.Subscribe(subj, func(msg *nats.Msg) { wc.sendRaw(msg.Data) })
		if err != nil {
			wc.sendErr("SUBSCRIBE_FAILED", err.Error())
			return
		}
		wc.addSub(sub)
	}
	wc.send(map[string]any{"type": "subscribed", "channel": "private", "user_id": userID})
}

func (wc *wsConn) writer() {
	for {
		select {
		case <-wc.ctx.Done():
			return
		case b := <-wc.sendCh:
			if err := wc.c.Write(wc.ctx, websocket.MessageText, b); err != nil {
				return
			}
		}
	}
}

func (wc *wsConn) send(v any) {
	b, _ := json.Marshal(v)
	wc.sendRaw(b)
}

func (wc *wsConn) sendRaw(b []byte) {
	select {
	case wc.sendCh <- b:
	default:
		wc.close(websocket.StatusPolicyViolation, "backpressure")
	}
}

func (wc *wsConn) sendErr(code, msg string) {
	wc.send(map[string]any{"type": "error", "error": map[string]any{"code": code, "message": msg}})
}

func (wc *wsConn) addSub(sub *nats.Subscription) {
	wc.subsMu.Lock()
	wc.subs = append(wc.subs, sub)
	wc.subsMu.Unlock()
}

func (wc *wsConn) close(status websocket.StatusCode, reason string) {
	wc.cancel()
	wc.subsMu.Lock()
	for _, s := range wc.subs {
		_ = s.Drain()
	}
	wc.subs = nil
	wc.subsMu.Unlock()
	_ = wc.c.Close(status, reason)
}

func seqFromMsg(b []byte) uint64 {
	var m map[string]any
	if json.Unmarshal(b, &m) != nil {
		return 0
	}
	if v, ok := m["global_seq"]; ok {
		return toU64(v)
	}
	if v, ok := m["globalSeq"]; ok {
		return toU64(v)
	}
	if v, ok := m["seq"]; ok {
		return toU64(v)
	}
	return 0
}

func toU64(v any) uint64 {
	switch t := v.(type) {
	case float64:
		if t < 0 {
			return 0
		}
		return uint64(t)
	case string:
		u, _ := strconv.ParseUint(t, 10, 64)
		return u
	default:
		return 0
	}
}

func parseDemoToken(tok string) (string, bool) {
	if strings.HasPrefix(tok, "Bearer ") {
		tok = strings.TrimPrefix(tok, "Bearer ")
	}
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

func getenv(k, def string) string {
	if v := os.Getenv(k); v != "" {
		return v
	}
	return def
}
