package main

import (
	"context"
	"encoding/json"
	"log"
	"net/http"
	"os"
	"os/signal"
	"syscall"
	"time"

	"github.com/jackc/pgx/v5/pgxpool"
	"github.com/nats-io/nats.go"
)

type errResp struct {
	Error map[string]any `json:"error"`
}

func main() {
	ctx, stop := signal.NotifyContext(context.Background(), syscall.SIGINT, syscall.SIGTERM)
	defer stop()

	serviceName := getenv("SERVICE_NAME", "gw-rest")
	addr := ":" + getenv("HTTP_PORT", "8081")
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
	mux.HandleFunc("/v1/orders", notImplemented)
	mux.HandleFunc("/v1/markets", notImplemented)
	mux.HandleFunc("/v1/positions", notImplemented)
	mux.HandleFunc("/v1/account/balance", notImplemented)

	srv := &http.Server{Addr: addr, Handler: mux, ReadHeaderTimeout: 3 * time.Second}
	go func() {
		<-ctx.Done()
		shutdownCtx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cancel()
		_ = srv.Shutdown(shutdownCtx)
	}()

	log.Printf("service=%s msg=starting addr=%s", serviceName, addr)
	if err := srv.ListenAndServe(); err != nil && err != http.ErrServerClosed {
		log.Fatal(err)
	}
}

func notImplemented(w http.ResponseWriter, _ *http.Request) {
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(http.StatusNotImplemented)
	_ = json.NewEncoder(w).Encode(errResp{Error: map[string]any{
		"code":    "NOT_IMPLEMENTED",
		"message": "Milestone 3 gateway skeleton endpoint.",
	}})
}

func getenv(k, def string) string {
	if v := os.Getenv(k); v != "" {
		return v
	}
	return def
}
