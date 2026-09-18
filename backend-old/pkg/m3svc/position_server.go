package m3svc

import (
	"context"
	"strconv"
	"strings"
	"time"

	sarvexv1 "github.com/sarvex/proto/gen/go/sarvex/v1"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
	"google.golang.org/protobuf/types/known/timestamppb"
)

func (s *positionServer) GetPosition(ctx context.Context, req *sarvexv1.GetPositionRequest) (*sarvexv1.UserPosition, error) {
	if strings.TrimSpace(req.GetUserId()) == "" || strings.TrimSpace(req.GetTicker()) == "" {
		return nil, status.Error(codes.InvalidArgument, "user_id and ticker are required")
	}
	row := s.pg.QueryRow(ctx, `SELECT user_id, ticker, net_qty, avg_cost_micro_usdc, realized_pnl_micro_usdc, last_global_seq, updated_at
FROM position.positions WHERE user_id=$1 AND ticker=$2`, req.GetUserId(), req.GetTicker())
	return scanPosition(row)
}

func (s *positionServer) ListPositions(ctx context.Context, req *sarvexv1.ListPositionsRequest) (*sarvexv1.ListPositionsResponse, error) {
	args := []any{req.GetUserId()}
	where := "user_id=$1"
	if !req.GetIncludeClosed() {
		where += " AND net_qty != 0"
	}
	q := `SELECT user_id, ticker, net_qty, avg_cost_micro_usdc, realized_pnl_micro_usdc, last_global_seq, updated_at
FROM position.positions WHERE ` + where + ` ORDER BY ticker ASC`
	rows, err := s.pg.Query(ctx, q, args...)
	if err != nil {
		return nil, mapPgErr(err)
	}
	defer rows.Close()
	resp := &sarvexv1.ListPositionsResponse{}
	for rows.Next() {
		p, err := scanPosition(rows)
		if err != nil {
			return nil, err
		}
		resp.Positions = append(resp.Positions, p)
	}
	return resp, nil
}

func (s *positionServer) ListPositionsByContract(ctx context.Context, req *sarvexv1.ListPositionsByContractRequest) (*sarvexv1.ListPositionsResponse, error) {
	limit := int(req.GetLimit())
	if limit <= 0 || limit > 200 {
		limit = 100
	}
	args := []any{req.GetTicker(), int64(req.GetMinGlobalSeq())}
	where := "ticker=$1 AND last_global_seq >= $2"
	if !req.GetIncludeClosed() {
		where += " AND net_qty != 0"
	}
	if req.GetCursor() != "" {
		args = append(args, req.GetCursor())
		where += " AND user_id > $" + strconv.Itoa(len(args))
	}
	args = append(args, limit+1)
	q := `SELECT user_id, ticker, net_qty, avg_cost_micro_usdc, realized_pnl_micro_usdc, last_global_seq, updated_at
FROM position.positions WHERE ` + where + ` ORDER BY user_id ASC LIMIT $` + strconv.Itoa(len(args))
	rows, err := s.pg.Query(ctx, q, args...)
	if err != nil {
		return nil, mapPgErr(err)
	}
	defer rows.Close()
	resp := &sarvexv1.ListPositionsResponse{}
	for rows.Next() {
		p, err := scanPosition(rows)
		if err != nil {
			return nil, err
		}
		resp.Positions = append(resp.Positions, p)
	}
	if len(resp.Positions) > limit {
		resp.NextCursor = resp.Positions[limit-1].GetUserId()
		resp.Positions = resp.Positions[:limit]
	}
	return resp, nil
}

func (s *positionServer) GetOpenInterest(ctx context.Context, req *sarvexv1.GetOpenInterestRequest) (*sarvexv1.OpenInterest, error) {
	var oi sarvexv1.OpenInterest
	err := s.pg.QueryRow(ctx, `SELECT ticker, total_open_long, total_open_short FROM position.open_interest WHERE ticker=$1`, req.GetTicker()).
		Scan(&oi.Ticker, &oi.TotalOpenLong, &oi.TotalOpenShort)
	if err != nil {
		return nil, mapPgErr(err)
	}
	return &oi, nil
}

func scanPosition(row interface{ Scan(dest ...any) error }) (*sarvexv1.UserPosition, error) {
	var p sarvexv1.UserPosition
	var ts time.Time
	if err := row.Scan(&p.UserId, &p.Ticker, &p.NetQty, &p.AvgCostMicroUsdc, &p.RealizedPnlMicroUsdc, &p.LastGlobalSeq, &ts); err != nil {
		return nil, mapPgErr(err)
	}
	p.UpdatedAt = timestamppb.New(ts)
	return &p, nil
}
