package m3svc

import (
	"context"
	"errors"
	"fmt"
	"strings"
	"time"

	"github.com/jackc/pgx/v5"
	"github.com/jackc/pgx/v5/pgtype"
	sarvexv1 "github.com/sarvex/proto/gen/go/sarvex/v1"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
	"google.golang.org/protobuf/types/known/emptypb"
	"google.golang.org/protobuf/types/known/structpb"
	"google.golang.org/protobuf/types/known/timestamppb"
)

func (s *refDataServer) GetContract(ctx context.Context, req *sarvexv1.GetContractRequest) (*sarvexv1.Contract, error) {
	if strings.TrimSpace(req.GetTicker()) == "" {
		return nil, status.Error(codes.InvalidArgument, "ticker is required")
	}
	c, err := s.fetchContract(ctx, req.GetTicker())
	if err != nil {
		if errors.Is(err, pgx.ErrNoRows) {
			return nil, status.Error(codes.NotFound, "contract not found")
		}
		return nil, mapPgErr(err)
	}
	return c, nil
}

func (s *refDataServer) ListContracts(ctx context.Context, req *sarvexv1.ListContractsRequest) (*sarvexv1.ListContractsResponse, error) {
	limit := int(req.GetLimit())
	if limit <= 0 || limit > 200 {
		limit = 50
	}

	args := []any{}
	where := []string{"1=1"}
	if req.GetState() != sarvexv1.ContractState_CONTRACT_STATE_UNSPECIFIED {
		args = append(args, contractStateDB(req.GetState()))
		where = append(where, "state = $"+itoa(len(args)))
	}
	if strings.TrimSpace(req.GetSeriesTicker()) != "" {
		args = append(args, req.GetSeriesTicker())
		where = append(where, "series_ticker = $"+itoa(len(args)))
	}
	if strings.TrimSpace(req.GetCursor()) != "" {
		args = append(args, req.GetCursor())
		where = append(where, "ticker > $"+itoa(len(args)))
	}
	args = append(args, limit+1)

	q := `SELECT ticker, event_ticker, series_ticker, kind, question, underlying, tick_size, min_price_ticks,
max_price_ticks, lower_bound_ticks, upper_bound_ticks, multiplier_micro_usdc, max_order_size,
position_limit_per_user, state, listed_at, open_at, close_at, expected_resolution_at,
settlement_source, oracle_policy, settlement_rule, close_global_seq
FROM refdata.contracts WHERE ` + strings.Join(where, " AND ") + ` ORDER BY ticker ASC LIMIT $` + itoa(len(args))

	rows, err := s.pg.Query(ctx, q, args...)
	if err != nil {
		return nil, mapPgErr(err)
	}
	defer rows.Close()

	resp := &sarvexv1.ListContractsResponse{}
	for rows.Next() {
		c, err := scanContract(rows)
		if err != nil {
			return nil, err
		}
		resp.Contracts = append(resp.Contracts, c)
	}
	if len(resp.Contracts) > limit {
		resp.NextCursor = resp.Contracts[limit-1].Ticker
		resp.Contracts = resp.Contracts[:limit]
	}
	return resp, nil
}

func (s *refDataServer) TransitionState(ctx context.Context, req *sarvexv1.TransitionStateRequest) (*emptypb.Empty, error) {
	if strings.TrimSpace(req.GetTicker()) == "" || req.GetNewState() == sarvexv1.ContractState_CONTRACT_STATE_UNSPECIFIED {
		return nil, status.Error(codes.InvalidArgument, "ticker and new_state are required")
	}

	tx, err := s.pg.BeginTx(ctx, pgx.TxOptions{})
	if err != nil {
		return nil, mapPgErr(err)
	}
	defer tx.Rollback(ctx)

	var oldState string
	err = tx.QueryRow(ctx, `SELECT state::text FROM refdata.contracts WHERE ticker=$1 FOR UPDATE`, req.GetTicker()).Scan(&oldState)
	if errors.Is(err, pgx.ErrNoRows) {
		return nil, status.Error(codes.NotFound, "contract not found")
	}
	if err != nil {
		return nil, mapPgErr(err)
	}

	newState := contractStateDB(req.GetNewState())
	_, err = tx.Exec(ctx, `UPDATE refdata.contracts SET state=$1::refdata.contract_state, updated_at=now() WHERE ticker=$2`, newState, req.GetTicker())
	if err != nil {
		return nil, mapPgErr(err)
	}

	_, err = tx.Exec(ctx,
		`INSERT INTO refdata.contract_state_history (ticker, old_state, new_state, reason, changed_by)
VALUES ($1,$2::refdata.contract_state,$3::refdata.contract_state,$4,$5)`,
		req.GetTicker(), oldState, newState, req.GetReason(), "service:refdata-svc",
	)
	if err != nil {
		return nil, mapPgErr(err)
	}
	if err := tx.Commit(ctx); err != nil {
		return nil, mapPgErr(err)
	}
	return &emptypb.Empty{}, nil
}

func (s *refDataServer) UpsertContract(ctx context.Context, req *sarvexv1.UpsertContractRequest) (*sarvexv1.Contract, error) {
	c := req.GetContract()
	if c == nil || strings.TrimSpace(c.GetTicker()) == "" || strings.TrimSpace(c.GetEventTicker()) == "" || strings.TrimSpace(c.GetSeriesTicker()) == "" {
		return nil, status.Error(codes.InvalidArgument, "contract.ticker/event_ticker/series_ticker required")
	}
	if c.GetTickSize() <= 0 || c.GetMaxOrderSize() <= 0 || c.GetPositionLimitPerUser() <= 0 {
		return nil, status.Error(codes.InvalidArgument, "invalid positive limits")
	}

	settlementRule := map[string]any{}
	if c.GetSettlementRule() != nil {
		settlementRule = c.GetSettlementRule().AsMap()
	}
	settlementRuleJSON, err := structToJSON(settlementRule)
	if err != nil {
		return nil, status.Errorf(codes.InvalidArgument, "invalid settlement_rule: %v", err)
	}

	_, err = s.pg.Exec(ctx,
		`INSERT INTO refdata.contracts (
ticker, event_ticker, series_ticker, kind, question, underlying,
tick_size, min_price_ticks, max_price_ticks, lower_bound_ticks, upper_bound_ticks,
multiplier_micro_usdc, max_order_size, position_limit_per_user, state,
listed_at, open_at, close_at, expected_resolution_at,
settlement_source, oracle_policy, settlement_rule, close_global_seq, updated_at
) VALUES (
$1,$2,$3,$4::refdata.contract_kind,$5,$6,
$7,$8,$9,NULLIF($10,0),NULLIF($11,0),
NULLIF($12,0),$13,$14,$15::refdata.contract_state,
$16,$17,$18,$19,
$20,$21,$22,$23,now()
)
ON CONFLICT (ticker) DO UPDATE SET
event_ticker=EXCLUDED.event_ticker,
series_ticker=EXCLUDED.series_ticker,
kind=EXCLUDED.kind,
question=EXCLUDED.question,
underlying=EXCLUDED.underlying,
tick_size=EXCLUDED.tick_size,
min_price_ticks=EXCLUDED.min_price_ticks,
max_price_ticks=EXCLUDED.max_price_ticks,
lower_bound_ticks=EXCLUDED.lower_bound_ticks,
upper_bound_ticks=EXCLUDED.upper_bound_ticks,
multiplier_micro_usdc=EXCLUDED.multiplier_micro_usdc,
max_order_size=EXCLUDED.max_order_size,
position_limit_per_user=EXCLUDED.position_limit_per_user,
state=EXCLUDED.state,
listed_at=EXCLUDED.listed_at,
open_at=EXCLUDED.open_at,
close_at=EXCLUDED.close_at,
expected_resolution_at=EXCLUDED.expected_resolution_at,
settlement_source=EXCLUDED.settlement_source,
oracle_policy=EXCLUDED.oracle_policy,
settlement_rule=EXCLUDED.settlement_rule,
close_global_seq=EXCLUDED.close_global_seq,
updated_at=now()`,
		c.GetTicker(), c.GetEventTicker(), c.GetSeriesTicker(), contractKindDB(c.GetKind()), c.GetQuestion(), c.GetUnderlying(),
		c.GetTickSize(), c.GetMinPriceTicks(), c.GetMaxPriceTicks(), c.GetLowerBoundTicks(), c.GetUpperBoundTicks(),
		c.GetMultiplierMicroUsdc(), c.GetMaxOrderSize(), c.GetPositionLimitPerUser(), contractStateDB(c.GetState()),
		tsOrNil(c.GetListedAt()), tsOrNil(c.GetOpenAt()), tsOrNil(c.GetCloseAt()), tsOrNil(c.GetExpectedResolutionAt()),
		c.GetSettlementSource(), defaultStr(c.GetOraclePolicy(), "ADMIN"), settlementRuleJSON, c.GetCloseGlobalSeq(),
	)
	if err != nil {
		return nil, mapPgErr(err)
	}

	out, err := s.fetchContract(ctx, c.GetTicker())
	if err != nil {
		return nil, mapPgErr(err)
	}
	return out, nil
}

func (s *refDataServer) GetEvent(ctx context.Context, req *sarvexv1.GetEventRequest) (*sarvexv1.Event, error) {
	if strings.TrimSpace(req.GetEventTicker()) == "" {
		return nil, status.Error(codes.InvalidArgument, "event_ticker required")
	}
	var e sarvexv1.Event
	var expected time.Time
	err := s.pg.QueryRow(ctx,
		`SELECT event_ticker, series_ticker, title, description, expected_resolution_at
FROM refdata.events WHERE event_ticker=$1`, req.GetEventTicker(),
	).Scan(&e.EventTicker, &e.SeriesTicker, &e.Title, &e.Description, &expected)
	if errors.Is(err, pgx.ErrNoRows) {
		return nil, status.Error(codes.NotFound, "event not found")
	}
	if err != nil {
		return nil, mapPgErr(err)
	}
	e.ExpectedResolutionAt = timestamppb.New(expected)
	return &e, nil
}

func (s *refDataServer) fetchContract(ctx context.Context, ticker string) (*sarvexv1.Contract, error) {
	row := s.pg.QueryRow(ctx,
		`SELECT ticker, event_ticker, series_ticker, kind, question, underlying, tick_size, min_price_ticks,
max_price_ticks, lower_bound_ticks, upper_bound_ticks, multiplier_micro_usdc, max_order_size,
position_limit_per_user, state, listed_at, open_at, close_at, expected_resolution_at,
settlement_source, oracle_policy, settlement_rule, close_global_seq
FROM refdata.contracts WHERE ticker=$1`,
		ticker,
	)
	return scanContract(row)
}

func scanContract(row interface{ Scan(dest ...any) error }) (*sarvexv1.Contract, error) {
	var c sarvexv1.Contract
	var kind, state string
	var listed, openAt, closeAt, expected pgtype.Timestamptz
	var settlementRule map[string]any
	var closeSeq uint64
	var lower, upper, mult *int64

	err := row.Scan(
		&c.Ticker, &c.EventTicker, &c.SeriesTicker, &kind, &c.Question, &c.Underlying,
		&c.TickSize, &c.MinPriceTicks, &c.MaxPriceTicks, &lower, &upper, &mult, &c.MaxOrderSize,
		&c.PositionLimitPerUser, &state, &listed, &openAt, &closeAt, &expected,
		&c.SettlementSource, &c.OraclePolicy, &settlementRule, &closeSeq,
	)
	if err != nil {
		return nil, err
	}
	c.Kind = contractKindProto(kind)
	c.State = contractStateProto(state)
	if lower != nil {
		c.LowerBoundTicks = *lower
	}
	if upper != nil {
		c.UpperBoundTicks = *upper
	}
	if mult != nil {
		c.MultiplierMicroUsdc = *mult
	}
	if listed.Valid {
		c.ListedAt = timestamppb.New(listed.Time)
	}
	if openAt.Valid {
		c.OpenAt = timestamppb.New(openAt.Time)
	}
	if closeAt.Valid {
		c.CloseAt = timestamppb.New(closeAt.Time)
	}
	if expected.Valid {
		c.ExpectedResolutionAt = timestamppb.New(expected.Time)
	}
	if settlementRule != nil {
		s, _ := structpb.NewStruct(settlementRule)
		c.SettlementRule = s
	}
	c.CloseGlobalSeq = closeSeq
	return &c, nil
}

func contractKindDB(v sarvexv1.ContractKind) string {
	if v == sarvexv1.ContractKind_CONTRACT_KIND_SCALAR {
		return "SCALAR"
	}
	return "BINARY"
}

func contractKindProto(v string) sarvexv1.ContractKind {
	if v == "SCALAR" {
		return sarvexv1.ContractKind_CONTRACT_KIND_SCALAR
	}
	if v == "BINARY" {
		return sarvexv1.ContractKind_CONTRACT_KIND_BINARY
	}
	return sarvexv1.ContractKind_CONTRACT_KIND_UNSPECIFIED
}

func contractStateDB(v sarvexv1.ContractState) string {
	switch v {
	case sarvexv1.ContractState_CONTRACT_STATE_DRAFT:
		return "DRAFT"
	case sarvexv1.ContractState_CONTRACT_STATE_LISTED:
		return "LISTED"
	case sarvexv1.ContractState_CONTRACT_STATE_OPEN:
		return "OPEN"
	case sarvexv1.ContractState_CONTRACT_STATE_CLOSED:
		return "CLOSED"
	case sarvexv1.ContractState_CONTRACT_STATE_RESOLVING:
		return "RESOLVING"
	case sarvexv1.ContractState_CONTRACT_STATE_SETTLED:
		return "SETTLED"
	case sarvexv1.ContractState_CONTRACT_STATE_CANCELLED:
		return "CANCELLED"
	case sarvexv1.ContractState_CONTRACT_STATE_HALTED:
		return "HALTED"
	default:
		return "DRAFT"
	}
}

func contractStateProto(v string) sarvexv1.ContractState {
	switch v {
	case "DRAFT":
		return sarvexv1.ContractState_CONTRACT_STATE_DRAFT
	case "LISTED":
		return sarvexv1.ContractState_CONTRACT_STATE_LISTED
	case "OPEN":
		return sarvexv1.ContractState_CONTRACT_STATE_OPEN
	case "CLOSED":
		return sarvexv1.ContractState_CONTRACT_STATE_CLOSED
	case "RESOLVING":
		return sarvexv1.ContractState_CONTRACT_STATE_RESOLVING
	case "SETTLED":
		return sarvexv1.ContractState_CONTRACT_STATE_SETTLED
	case "CANCELLED":
		return sarvexv1.ContractState_CONTRACT_STATE_CANCELLED
	case "HALTED":
		return sarvexv1.ContractState_CONTRACT_STATE_HALTED
	default:
		return sarvexv1.ContractState_CONTRACT_STATE_UNSPECIFIED
	}
}

func tsOrNil(ts *timestamppb.Timestamp) any {
	if ts == nil {
		return nil
	}
	return ts.AsTime()
}

func defaultStr(v, d string) string {
	if strings.TrimSpace(v) == "" {
		return d
	}
	return v
}

func itoa(v int) string {
	return fmt.Sprintf("%d", v)
}
