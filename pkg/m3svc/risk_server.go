package m3svc

import (
	"context"
	"errors"
	"math"
	"strings"

	"github.com/jackc/pgx/v5"
	sarvexv1 "github.com/sarvex/proto/gen/go/sarvex/v1"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
	"google.golang.org/protobuf/types/known/emptypb"
)

func (s *riskServer) PreTradeCheck(ctx context.Context, req *sarvexv1.PreTradeCheckRequest) (*sarvexv1.PreTradeCheckResponse, error) {
	if strings.TrimSpace(req.GetUserId()) == "" || strings.TrimSpace(req.GetTicker()) == "" {
		return nil, status.Error(codes.InvalidArgument, "user_id and ticker are required")
	}
	if req.GetCount() <= 0 {
		return reject("INVALID_QUANTITY", "count must be positive"), nil
	}

	limits, err := s.loadUserLimits(ctx, req.GetUserId())
	if err != nil {
		if errors.Is(err, pgx.ErrNoRows) {
			return reject("USER_LIMITS_NOT_FOUND", "user limits missing"), nil
		}
		return nil, mapPgErr(err)
	}
	contract, err := s.loadContract(ctx, req.GetTicker())
	if err != nil {
		if errors.Is(err, pgx.ErrNoRows) {
			return reject("CONTRACT_NOT_FOUND", "contract not found"), nil
		}
		return nil, mapPgErr(err)
	}

	if contract.state != "OPEN" {
		return reject("CONTRACT_NOT_OPEN", "contract is not open"), nil
	}
	if req.GetPriceTicks() != 0 && (req.GetPriceTicks() < contract.minPrice || req.GetPriceTicks() > contract.maxPrice) {
		return reject("INVALID_PRICE", "price out of range"), nil
	}
	if contract.tickSize > 0 && req.GetPriceTicks() > 0 && req.GetPriceTicks()%contract.tickSize != 0 {
		return reject("INVALID_TICK_ALIGNMENT", "price not aligned to tick size"), nil
	}
	if req.GetCount() > contract.maxOrderSize {
		return reject("MAX_ORDER_SIZE_EXCEEDED", "count exceeds contract max order size"), nil
	}

	requiredHold, err := computeRequiredHold(req, contract)
	if err != nil {
		return reject("INVALID_ORDER", err.Error()), nil
	}
	if requiredHold > limits.maxOrderSizeMicroUSDC {
		return reject("MAX_ORDER_NOTIONAL_EXCEEDED", "required hold exceeds max order limit"), nil
	}

	currentPos, err := s.currentPosition(ctx, req.GetUserId(), req.GetTicker())
	if err != nil {
		return nil, mapPgErr(err)
	}
	workingQty, err := s.workingOrderSignedQty(ctx, req.GetUserId(), req.GetTicker())
	if err != nil {
		return nil, mapPgErr(err)
	}
	thisQty := signedQty(req.GetAction(), req.GetCount())
	projected := currentPos + workingQty + thisQty

	limit, err := s.positionLimit(ctx, req.GetUserId(), req.GetTicker(), contract.positionLimitPerUser)
	if err != nil {
		return nil, mapPgErr(err)
	}
	if abs(projected) > limit {
		return reject("POSITION_LIMIT_EXCEEDED", "projected position exceeds limit"), nil
	}

	return &sarvexv1.PreTradeCheckResponse{
		Approved:              true,
		RequiredHoldMicroUsdc: requiredHold,
		ProjectedPosition:     projected,
	}, nil
}

func (s *riskServer) GetUserLimits(ctx context.Context, req *sarvexv1.GetUserLimitsRequest) (*sarvexv1.UserLimits, error) {
	if strings.TrimSpace(req.GetUserId()) == "" {
		return nil, status.Error(codes.InvalidArgument, "user_id required")
	}
	ul, err := s.loadUserLimits(ctx, req.GetUserId())
	if errors.Is(err, pgx.ErrNoRows) {
		return nil, status.Error(codes.NotFound, "user limits not found")
	}
	if err != nil {
		return nil, mapPgErr(err)
	}

	rows, err := s.pg.Query(ctx, `SELECT ticker, max_qty FROM risk.contract_position_limits WHERE user_id=$1`, req.GetUserId())
	if err != nil {
		return nil, mapPgErr(err)
	}
	defer rows.Close()

	m := map[string]int64{}
	for rows.Next() {
		var ticker string
		var maxQty int64
		if err := rows.Scan(&ticker, &maxQty); err != nil {
			return nil, mapPgErr(err)
		}
		m[ticker] = maxQty
	}

	return &sarvexv1.UserLimits{
		UserId:                   req.GetUserId(),
		KycTier:                  ul.kycTier,
		MaxOrderSizeMicroUsdc:    ul.maxOrderSizeMicroUSDC,
		DailyLossLimitMicroUsdc:  ul.dailyLossLimitMicroUSDC,
		PerContractPositionLimit: m,
	}, nil
}

func (s *riskServer) UpdateUserLimits(ctx context.Context, req *sarvexv1.UpdateUserLimitsRequest) (*emptypb.Empty, error) {
	l := req.GetLimits()
	if l == nil || strings.TrimSpace(l.GetUserId()) == "" {
		return nil, status.Error(codes.InvalidArgument, "limits.user_id required")
	}
	if l.GetMaxOrderSizeMicroUsdc() <= 0 || l.GetDailyLossLimitMicroUsdc() <= 0 {
		return nil, status.Error(codes.InvalidArgument, "limits must be positive")
	}

	tx, err := s.pg.BeginTx(ctx, pgx.TxOptions{})
	if err != nil {
		return nil, mapPgErr(err)
	}
	defer tx.Rollback(ctx)

	_, err = tx.Exec(ctx,
		`INSERT INTO risk.user_limits (user_id, kyc_tier, max_order_size_micro_usdc, daily_loss_limit_micro_usdc, updated_at)
VALUES ($1,$2,$3,$4,now())
ON CONFLICT (user_id) DO UPDATE SET
kyc_tier=EXCLUDED.kyc_tier,
max_order_size_micro_usdc=EXCLUDED.max_order_size_micro_usdc,
daily_loss_limit_micro_usdc=EXCLUDED.daily_loss_limit_micro_usdc,
updated_at=now()`,
		l.GetUserId(), l.GetKycTier(), l.GetMaxOrderSizeMicroUsdc(), l.GetDailyLossLimitMicroUsdc(),
	)
	if err != nil {
		return nil, mapPgErr(err)
	}

	_, err = tx.Exec(ctx, `DELETE FROM risk.contract_position_limits WHERE user_id=$1`, l.GetUserId())
	if err != nil {
		return nil, mapPgErr(err)
	}
	for ticker, maxQty := range l.GetPerContractPositionLimit() {
		if strings.TrimSpace(ticker) == "" || maxQty <= 0 {
			continue
		}
		_, err = tx.Exec(ctx,
			`INSERT INTO risk.contract_position_limits (user_id, ticker, max_qty) VALUES ($1,$2,$3)`,
			l.GetUserId(), ticker, maxQty,
		)
		if err != nil {
			return nil, mapPgErr(err)
		}
	}

	if err := tx.Commit(ctx); err != nil {
		return nil, mapPgErr(err)
	}
	return &emptypb.Empty{}, nil
}

type userLimitsRow struct {
	kycTier                 int32
	maxOrderSizeMicroUSDC   int64
	dailyLossLimitMicroUSDC int64
}

type contractRow struct {
	kind                 string
	state                string
	tickSize             int64
	minPrice             int64
	maxPrice             int64
	lowerBound           int64
	upperBound           int64
	multiplier           int64
	maxOrderSize         int64
	positionLimitPerUser int64
}

func (s *riskServer) loadUserLimits(ctx context.Context, userID string) (*userLimitsRow, error) {
	var ul userLimitsRow
	err := s.pg.QueryRow(ctx,
		`SELECT kyc_tier, max_order_size_micro_usdc, daily_loss_limit_micro_usdc
FROM risk.user_limits WHERE user_id=$1`,
		userID,
	).Scan(&ul.kycTier, &ul.maxOrderSizeMicroUSDC, &ul.dailyLossLimitMicroUSDC)
	if err != nil {
		return nil, err
	}
	return &ul, nil
}

func (s *riskServer) loadContract(ctx context.Context, ticker string) (*contractRow, error) {
	var c contractRow
	err := s.pg.QueryRow(ctx,
		`SELECT kind::text, state::text, tick_size, min_price_ticks, max_price_ticks,
COALESCE(lower_bound_ticks,0), COALESCE(upper_bound_ticks,0), COALESCE(multiplier_micro_usdc,0),
max_order_size, position_limit_per_user
FROM refdata.contracts WHERE ticker=$1`,
		ticker,
	).Scan(&c.kind, &c.state, &c.tickSize, &c.minPrice, &c.maxPrice, &c.lowerBound, &c.upperBound, &c.multiplier, &c.maxOrderSize, &c.positionLimitPerUser)
	if err != nil {
		return nil, err
	}
	return &c, nil
}

func computeRequiredHold(req *sarvexv1.PreTradeCheckRequest, c *contractRow) (int64, error) {
	price := req.GetPriceTicks()
	count := req.GetCount()
	if price <= 0 {
		return 0, errors.New("market price (0) not supported in demo risk calculator")
	}

	if c.kind == "BINARY" {
		if req.GetAction() == sarvexv1.Action_ACTION_BUY {
			return price * count * 10000, nil
		}
		if req.GetAction() == sarvexv1.Action_ACTION_SELL {
			return (100 - price) * count * 10000, nil
		}
		return 0, errors.New("invalid action")
	}

	if c.kind == "SCALAR" {
		switch req.GetSide() {
		case sarvexv1.Side_SIDE_LONG:
			if price < c.lowerBound {
				return 0, errors.New("price below scalar lower bound")
			}
			return (price - c.lowerBound) * count * c.multiplier, nil
		case sarvexv1.Side_SIDE_SHORT:
			if price > c.upperBound {
				return 0, errors.New("price above scalar upper bound")
			}
			return (c.upperBound - price) * count * c.multiplier, nil
		default:
			return 0, errors.New("scalar side must be LONG or SHORT")
		}
	}
	return 0, errors.New("unknown contract kind")
}

func (s *riskServer) currentPosition(ctx context.Context, userID, ticker string) (int64, error) {
	var qty int64
	err := s.pg.QueryRow(ctx,
		`SELECT COALESCE(net_qty,0) FROM position.positions WHERE user_id=$1 AND ticker=$2`,
		userID, ticker,
	).Scan(&qty)
	if errors.Is(err, pgx.ErrNoRows) {
		return 0, nil
	}
	return qty, err
}

func (s *riskServer) workingOrderSignedQty(ctx context.Context, userID, ticker string) (int64, error) {
	var buyQty, sellQty int64
	err := s.pg.QueryRow(ctx,
		`SELECT COALESCE(SUM(CASE WHEN side IN ('YES','LONG') THEN total_qty ELSE 0 END),0),
COALESCE(SUM(CASE WHEN side IN ('NO','SHORT') THEN total_qty ELSE 0 END),0)
FROM risk.working_orders_summary
WHERE user_id=$1 AND ticker=$2`,
		userID, ticker,
	).Scan(&buyQty, &sellQty)
	if err != nil {
		return 0, err
	}
	return buyQty - sellQty, nil
}

func signedQty(action sarvexv1.Action, qty int64) int64 {
	if action == sarvexv1.Action_ACTION_SELL {
		return -qty
	}
	return qty
}

func (s *riskServer) positionLimit(ctx context.Context, userID, ticker string, fallback int64) (int64, error) {
	var maxQty int64
	err := s.pg.QueryRow(ctx,
		`SELECT max_qty FROM risk.contract_position_limits WHERE user_id=$1 AND ticker=$2`,
		userID, ticker,
	).Scan(&maxQty)
	if errors.Is(err, pgx.ErrNoRows) {
		return fallback, nil
	}
	if err != nil {
		return 0, err
	}
	return maxQty, nil
}

func reject(code, reason string) *sarvexv1.PreTradeCheckResponse {
	return &sarvexv1.PreTradeCheckResponse{Approved: false, RejectCode: code, RejectReason: reason}
}

func abs(v int64) int64 {
	return int64(math.Abs(float64(v)))
}
