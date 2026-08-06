package m3svc

import "testing"

func TestRealizedPnLDeltaBinaryLongClose(t *testing.T) {
	got := realizedPnLDelta(positionContract{kind: "BINARY"}, 10, 52*10000, -10, 49)
	if want := int64(-300_000); got != want {
		t.Fatalf("realized=%d want=%d", got, want)
	}
}

func TestRealizedPnLDeltaScalarLongClose(t *testing.T) {
	contract := positionContract{kind: "SCALAR", multiplierMicroUSDC: 1_000_000}
	got := realizedPnLDelta(contract, 5, 780*10000, -5, 710)
	if want := int64(-350_000_000); got != want {
		t.Fatalf("realized=%d want=%d", got, want)
	}
}

func TestRealizedPnLDeltaScalarShortClose(t *testing.T) {
	contract := positionContract{kind: "SCALAR", multiplierMicroUSDC: 1_000_000}
	got := realizedPnLDelta(contract, -5, 710*10000, 5, 680)
	if want := int64(150_000_000); got != want {
		t.Fatalf("realized=%d want=%d", got, want)
	}
}
