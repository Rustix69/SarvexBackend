package m3svc

import (
	"testing"

	sarvexv1 "github.com/sarvex/proto/gen/go/sarvex/v1"
)

func TestFillHoldAmountBinaryUsesActionSpecificCollateral(t *testing.T) {
	contract := &sarvexv1.Contract{Kind: sarvexv1.ContractKind_CONTRACT_KIND_BINARY}

	buy, err := fillHoldAmount(contract, "YES", "BUY", 48, 10)
	if err != nil {
		t.Fatalf("buy hold: %v", err)
	}
	if want := int64(4_800_000); buy != want {
		t.Fatalf("buy hold=%d want=%d", buy, want)
	}

	sell, err := fillHoldAmount(contract, "YES", "SELL", 48, 10)
	if err != nil {
		t.Fatalf("sell hold: %v", err)
	}
	if want := int64(5_200_000); sell != want {
		t.Fatalf("sell hold=%d want=%d", sell, want)
	}
}

func TestFillHoldAmountScalarUsesPositionDirection(t *testing.T) {
	contract := &sarvexv1.Contract{
		Kind:                 sarvexv1.ContractKind_CONTRACT_KIND_SCALAR,
		LowerBoundTicks:      200,
		UpperBoundTicks:      800,
		MultiplierMicroUsdc:  1_000,
	}

	long, err := fillHoldAmount(contract, "LONG", "BUY", 450, 2)
	if err != nil {
		t.Fatalf("long hold: %v", err)
	}
	if want := int64(500_000); long != want {
		t.Fatalf("long hold=%d want=%d", long, want)
	}

	short, err := fillHoldAmount(contract, "SHORT", "BUY", 450, 2)
	if err != nil {
		t.Fatalf("short hold: %v", err)
	}
	if want := int64(700_000); short != want {
		t.Fatalf("short hold=%d want=%d", short, want)
	}
}
