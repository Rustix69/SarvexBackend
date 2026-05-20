package m3svc

import (
	"fmt"
	"os"
	"strconv"
)

type Config struct {
	ServiceName string
	GRPCPort    int
	HTTPPort    int
	PostgresURL string
	NATSURL     string
	MatchingAddr string
	RequireDB   bool
	RequireNATS bool
}

func LoadConfig(defaultGRPCPort, defaultHTTPPort int) (Config, error) {
	service := getenv("SERVICE_NAME", "unknown-svc")
	grpcPort, err := getenvInt("GRPC_PORT", defaultGRPCPort)
	if err != nil {
		return Config{}, err
	}
	httpPort, err := getenvInt("HTTP_PORT", defaultHTTPPort)
	if err != nil {
		return Config{}, err
	}

	cfg := Config{
		ServiceName: service,
		GRPCPort:    grpcPort,
		HTTPPort:    httpPort,
		PostgresURL: getenv("POSTGRES_URL", ""),
		NATSURL:     getenv("NATS_URL", ""),
		MatchingAddr: getenv("MATCHING_ENGINE_ADDR", "me-core:50051"),
		RequireDB:   getenv("REQUIRE_DB", "true") == "true",
		RequireNATS: getenv("REQUIRE_NATS", "true") == "true",
	}
	return cfg, nil
}

func getenv(k, def string) string {
	if v := os.Getenv(k); v != "" {
		return v
	}
	return def
}

func getenvInt(k string, def int) (int, error) {
	raw := getenv(k, fmt.Sprintf("%d", def))
	v, err := strconv.Atoi(raw)
	if err != nil {
		return 0, fmt.Errorf("invalid %s: %w", k, err)
	}
	return v, nil
}
