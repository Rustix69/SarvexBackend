package main

import (
	"context"
	"log"
	"os"
	"os/signal"
	"syscall"

	"github.com/sarvex/proto/pkg/m3svc"
)

func main() {
	ctx, stop := signal.NotifyContext(context.Background(), syscall.SIGINT, syscall.SIGTERM)
	defer stop()

	cfg, err := m3svc.LoadConfig(50051, 8080)
	if err != nil {
		log.Fatal(err)
	}
	role := getenv("SERVICE_ROLE", "")
	if role == "" {
		log.Fatal("SERVICE_ROLE is required")
	}

	if err := m3svc.RunGRPC(ctx, cfg, role); err != nil {
		log.Fatal(err)
	}
}

func getenv(k, def string) string {
	if v := os.Getenv(k); v != "" {
		return v
	}
	return def
}
