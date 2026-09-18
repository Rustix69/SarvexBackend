package main

import (
	"log"
	"net"

	sarvexv1 "github.com/sarvex/proto/gen/go/sarvex/v1"
	"google.golang.org/grpc"
)

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

func main() {
	lis, err := net.Listen("tcp", ":50051")
	if err != nil {
		log.Fatalf("listen: %v", err)
	}

	server := grpc.NewServer()
	sarvexv1.RegisterLedgerServer(server, &ledgerServer{})
	sarvexv1.RegisterMatchingEngineServer(server, &matchingEngineServer{})
	sarvexv1.RegisterOracleServer(server, &oracleServer{})
	sarvexv1.RegisterOrderRouterServer(server, &orderRouterServer{})
	sarvexv1.RegisterPositionServer(server, &positionServer{})
	sarvexv1.RegisterRefDataServer(server, &refDataServer{})
	sarvexv1.RegisterRiskServer(server, &riskServer{})
	sarvexv1.RegisterSettlementServer(server, &settlementServer{})

	log.Printf("proto stub server listening on %s", lis.Addr())
	if err := server.Serve(lis); err != nil {
		log.Fatalf("serve: %v", err)
	}
}
