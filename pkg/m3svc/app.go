package m3svc

import (
	"context"
	"errors"
	"fmt"
	"log"
	"net"
	"net/http"
	"sync/atomic"
	"time"

	"github.com/jackc/pgx/v5/pgxpool"
	"github.com/nats-io/nats.go"
	sarvexv1 "github.com/sarvex/proto/gen/go/sarvex/v1"
	"google.golang.org/grpc"
)

type App struct {
	cfg     Config
	ready   atomic.Bool
	pg      *pgxpool.Pool
	nc      *nats.Conn
	grpcSrv *grpc.Server
	httpSrv *http.Server
}

type ledgerServer struct {
	sarvexv1.UnimplementedLedgerServer
}
type matchingEngineServer struct {
	sarvexv1.UnimplementedMatchingEngineServer
}
type oracleServer struct {
	sarvexv1.UnimplementedOracleServer
}
type orderRouterServer struct {
	sarvexv1.UnimplementedOrderRouterServer
}
type positionServer struct {
	sarvexv1.UnimplementedPositionServer
}
type refDataServer struct {
	sarvexv1.UnimplementedRefDataServer
}
type riskServer struct {
	sarvexv1.UnimplementedRiskServer
}
type settlementServer struct {
	sarvexv1.UnimplementedSettlementServer
}

func RunGRPC(ctx context.Context, cfg Config, role string) error {
	app := &App{cfg: cfg}
	if err := app.connectDependencies(ctx); err != nil {
		return err
	}

	mux := http.NewServeMux()
	mux.HandleFunc("/healthz", func(w http.ResponseWriter, _ *http.Request) {
		w.WriteHeader(http.StatusOK)
		_, _ = w.Write([]byte("ok"))
	})
	mux.HandleFunc("/readyz", func(w http.ResponseWriter, _ *http.Request) {
		if !app.ready.Load() {
			w.WriteHeader(http.StatusServiceUnavailable)
			_, _ = w.Write([]byte("not ready"))
			return
		}
		w.WriteHeader(http.StatusOK)
		_, _ = w.Write([]byte("ready"))
	})
	app.httpSrv = &http.Server{Addr: fmt.Sprintf(":%d", cfg.HTTPPort), Handler: mux, ReadHeaderTimeout: 3 * time.Second}

	go func() {
		log.Printf("service=%s component=http msg=starting addr=%s", cfg.ServiceName, app.httpSrv.Addr)
		if err := app.httpSrv.ListenAndServe(); err != nil && !errors.Is(err, http.ErrServerClosed) {
			log.Printf("service=%s component=http level=error err=%v", cfg.ServiceName, err)
		}
	}()

	lis, err := net.Listen("tcp", fmt.Sprintf(":%d", cfg.GRPCPort))
	if err != nil {
		return fmt.Errorf("listen grpc: %w", err)
	}

	app.grpcSrv = grpc.NewServer()
	registerByRole(app.grpcSrv, role)
	app.ready.Store(true)
	log.Printf("service=%s component=grpc msg=starting addr=:%d role=%s", cfg.ServiceName, cfg.GRPCPort, role)

	go func() {
		<-ctx.Done()
		app.ready.Store(false)
		shutdownCtx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cancel()
		if app.httpSrv != nil {
			_ = app.httpSrv.Shutdown(shutdownCtx)
		}
		if app.grpcSrv != nil {
			app.grpcSrv.GracefulStop()
		}
		if app.pg != nil {
			app.pg.Close()
		}
		if app.nc != nil {
			app.nc.Close()
		}
	}()

	if err := app.grpcSrv.Serve(lis); err != nil {
		return fmt.Errorf("serve grpc: %w", err)
	}
	return nil
}

func registerByRole(server *grpc.Server, role string) {
	switch role {
	case "ledger":
		sarvexv1.RegisterLedgerServer(server, &ledgerServer{})
	case "matching":
		sarvexv1.RegisterMatchingEngineServer(server, &matchingEngineServer{})
	case "oracle":
		sarvexv1.RegisterOracleServer(server, &oracleServer{})
	case "order-router":
		sarvexv1.RegisterOrderRouterServer(server, &orderRouterServer{})
	case "position":
		sarvexv1.RegisterPositionServer(server, &positionServer{})
	case "refdata":
		sarvexv1.RegisterRefDataServer(server, &refDataServer{})
	case "risk":
		sarvexv1.RegisterRiskServer(server, &riskServer{})
	case "settlement":
		sarvexv1.RegisterSettlementServer(server, &settlementServer{})
	}
}

func (a *App) connectDependencies(ctx context.Context) error {
	if a.cfg.RequireDB {
		if a.cfg.PostgresURL == "" {
			return errors.New("POSTGRES_URL is required")
		}
		pool, err := pgxpool.New(ctx, a.cfg.PostgresURL)
		if err != nil {
			return fmt.Errorf("connect postgres: %w", err)
		}
		if err := pool.Ping(ctx); err != nil {
			pool.Close()
			return fmt.Errorf("ping postgres: %w", err)
		}
		a.pg = pool
	}

	if a.cfg.RequireNATS {
		if a.cfg.NATSURL == "" {
			return errors.New("NATS_URL is required")
		}
		nc, err := nats.Connect(a.cfg.NATSURL, nats.Name(a.cfg.ServiceName))
		if err != nil {
			return fmt.Errorf("connect nats: %w", err)
		}
		a.nc = nc
	}
	return nil
}
