package m3svc

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"fmt"
	"strconv"
	"strings"
	"sync"
	"time"

	"github.com/jackc/pgx/v5"
	"github.com/jackc/pgx/v5/pgconn"
	"github.com/jackc/pgx/v5/pgxpool"
	sarvexv1 "github.com/sarvex/proto/gen/go/sarvex/v1"
	"google.golang.org/grpc"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/credentials/insecure"
	"google.golang.org/grpc/status"
	"google.golang.org/protobuf/types/known/timestamppb"
)

var sfMu sync.Mutex
var sfLastMs int64
var sfSeq int64

const (
	meFlagIOC        uint32 = 1 << 0
	meFlagFOK        uint32 = 1 << 1
	meFlagPostOnly   uint32 = 1 << 2
	meFlagReduceOnly uint32 = 1 << 3
)

func nextOrderID() string {
	sfMu.Lock()
	defer sfMu.Unlock()
	nowMs := time.Now().UTC().UnixMilli()
	if nowMs == sfLastMs {
		sfSeq++
		if sfSeq >= 4096 {
			for nowMs == sfLastMs {
				time.Sleep(50 * time.Microsecond)
				nowMs = time.Now().UTC().UnixMilli()
			}
			sfSeq = 0
		}
	} else {
		sfSeq = 0
		sfLastMs = nowMs
	}
	epochMs := time.Date(2026, 1, 1, 0, 0, 0, 0, time.UTC).UnixMilli()
	v := ((nowMs - epochMs) << 22) | sfSeq
	return strconv.FormatInt(v, 10)
}

func (s *orderRouterServer) SubmitOrder(ctx context.Context, req *sarvexv1.SubmitOrderRequest) (*sarvexv1.SubmitOrderResponse, error) {
	if strings.TrimSpace(req.GetUserId()) == "" || strings.TrimSpace(req.GetTicker()) == "" || req.GetCount() <= 0 {
		return nil, status.Error(codes.InvalidArgument, "user_id, ticker, count are required")
	}
	if req.GetClientOrderId() == "" {
		req.ClientOrderId = fmt.Sprintf("coid_%d", time.Now().UnixNano())
	}
	if req.GetPriceTicks() == 0 {
		req.Tif = sarvexv1.TimeInForce_TIME_IN_FORCE_IOC
	}

	orderID := nextOrderID()
	err := s.pg.QueryRow(ctx, `INSERT INTO orders.orders (
order_id, client_order_id, user_id, ticker, side, action, price_ticks, count, filled_count, remaining_count, tif, post_only, reduce_only, stp, status, created_at, updated_at, expires_at
) VALUES (
$1,$2,$3,$4,$5::orders.order_side,$6::orders.order_action,$7,$8,0,$8,$9::orders.time_in_force,$10,$11,$12,'PENDING',now(),now(),$13
) RETURNING order_id`,
		orderID, req.GetClientOrderId(), req.GetUserId(), req.GetTicker(), sideDB(req.GetSide()), actionDB(req.GetAction()), req.GetPriceTicks(), req.GetCount(),
		tifDB(req.GetTif()), req.GetPostOnly(), req.GetReduceOnly(), stpDB(req.GetStp()), tsOrNil(req.GetExpiresAt()),
	).Scan(&orderID)
	if err != nil {
		var pgErr *pgconn.PgError
		if errors.As(err, &pgErr) && pgErr.Code == "23505" {
			existing, gerr := s.GetOrder(ctx, &sarvexv1.GetOrderRequest{UserId: req.GetUserId(), Key: &sarvexv1.GetOrderRequest_ClientOrderId{ClientOrderId: req.GetClientOrderId()}})
			if gerr != nil {
				return nil, mapPgErr(err)
			}
			return &sarvexv1.SubmitOrderResponse{Order: existing}, nil
		}
		return nil, mapPgErr(err)
	}

	refSrv := &refDataServer{pg: s.pg}
	contract, err := refSrv.fetchContract(ctx, req.GetTicker())
	if err != nil {
		_ = markRejected(ctx, s.pg, orderID, "CONTRACT_NOT_FOUND", "contract not found")
		s.publishExecEvents(orderID, req.GetUserId(), req.GetTicker(), "ORDER_REJECTED", "CONTRACT_NOT_FOUND")
		return &sarvexv1.SubmitOrderResponse{RejectCode: "CONTRACT_NOT_FOUND", RejectReason: "contract not found"}, nil
	}
	if contract.GetState() != sarvexv1.ContractState_CONTRACT_STATE_OPEN {
		_ = markRejected(ctx, s.pg, orderID, "CONTRACT_NOT_OPEN", "contract not open")
		s.publishExecEvents(orderID, req.GetUserId(), req.GetTicker(), "ORDER_REJECTED", "CONTRACT_NOT_OPEN")
		return &sarvexv1.SubmitOrderResponse{RejectCode: "CONTRACT_NOT_OPEN", RejectReason: "contract not open"}, nil
	}
	execReq, rejectCode, rejectReason := executionOrderRequest(req, contract)
	if rejectCode != "" {
		_ = markRejected(ctx, s.pg, orderID, rejectCode, rejectReason)
		s.publishExecEvents(orderID, req.GetUserId(), req.GetTicker(), "ORDER_REJECTED", rejectCode)
		return &sarvexv1.SubmitOrderResponse{RejectCode: rejectCode, RejectReason: rejectReason}, nil
	}
	if req.GetReduceOnly() {
		rejectCode, rejectReason, err := s.validateReduceOnly(ctx, execReq)
		if err != nil {
			return nil, err
		}
		if rejectCode != "" {
			_ = markRejected(ctx, s.pg, orderID, rejectCode, rejectReason)
			s.publishExecEvents(orderID, req.GetUserId(), req.GetTicker(), "ORDER_REJECTED", rejectCode)
			return &sarvexv1.SubmitOrderResponse{RejectCode: rejectCode, RejectReason: rejectReason}, nil
		}
	}

	risk := &riskServer{pg: s.pg}
	riskResp, err := risk.PreTradeCheck(ctx, &sarvexv1.PreTradeCheckRequest{
		UserId: execReq.GetUserId(), Ticker: execReq.GetTicker(), Side: execReq.GetSide(), Action: execReq.GetAction(), PriceTicks: execReq.GetPriceTicks(), Count: execReq.GetCount(),
	})
	if err != nil {
		return nil, err
	}
	if !riskResp.GetApproved() {
		_ = markRejected(ctx, s.pg, orderID, riskResp.GetRejectCode(), riskResp.GetRejectReason())
		s.publishExecEvents(orderID, req.GetUserId(), req.GetTicker(), "ORDER_REJECTED", riskResp.GetRejectCode())
		return &sarvexv1.SubmitOrderResponse{RejectCode: riskResp.GetRejectCode(), RejectReason: riskResp.GetRejectReason()}, nil
	}

	ledger := &ledgerServer{pg: s.pg}
	holdResp, err := ledger.PlaceHold(ctx, &sarvexv1.PlaceHoldRequest{
		IdempotencyKey:  "place:" + orderID,
		UserId:          req.GetUserId(),
		AmountMicroUsdc: riskResp.GetRequiredHoldMicroUsdc(),
		Reason:          "ORDER_SUBMIT",
	})
	if err != nil {
		_ = markRejected(ctx, s.pg, orderID, "INSUFFICIENT_FUNDS", "hold placement failed")
		s.publishExecEvents(orderID, req.GetUserId(), req.GetTicker(), "ORDER_REJECTED", "INSUFFICIENT_FUNDS")
		return &sarvexv1.SubmitOrderResponse{RejectCode: "INSUFFICIENT_FUNDS", RejectReason: "hold placement failed"}, nil
	}
	_, _ = s.pg.Exec(ctx, `UPDATE orders.orders SET hold_id=$1, updated_at=now() WHERE order_id=$2`, holdResp.GetHoldId(), orderID)

	meResp, meErr := s.submitToME(ctx, execReq, orderID, holdResp.GetHoldId())
	if meErr != nil {
		if status.Code(meErr) == codes.ResourceExhausted {
			_, _ = ledger.ReleaseHold(ctx, &sarvexv1.ReleaseHoldRequest{IdempotencyKey: "release:" + orderID + ":QUEUE_FULL", HoldId: holdResp.GetHoldId(), AmountMicroUsdc: riskResp.GetRequiredHoldMicroUsdc(), ReasonCode: "ENGINE_QUEUE_FULL"})
			_ = markRejected(ctx, s.pg, orderID, "ENGINE_QUEUE_FULL", "matching queue full")
			s.publishExecEvents(orderID, req.GetUserId(), req.GetTicker(), "ORDER_REJECTED", "ENGINE_QUEUE_FULL")
			return &sarvexv1.SubmitOrderResponse{RejectCode: "ENGINE_QUEUE_FULL", RejectReason: "matching queue full"}, nil
		}
		return &sarvexv1.SubmitOrderResponse{
			RejectCode:   "ACK_UNKNOWN",
			RejectReason: "matching outcome unknown; order left pending",
			Order:        mustGetOrder(ctx, s.pg, req.GetUserId(), orderID),
		}, nil
	}
	if !meResp.GetAccepted() {
		_, _ = ledger.ReleaseHold(ctx, &sarvexv1.ReleaseHoldRequest{IdempotencyKey: "release:" + orderID + ":ME_REJECT", HoldId: holdResp.GetHoldId(), AmountMicroUsdc: riskResp.GetRequiredHoldMicroUsdc(), ReasonCode: meResp.GetRejectCode()})
		_ = markRejected(ctx, s.pg, orderID, meResp.GetRejectCode(), "matching rejected order")
		s.publishExecEvents(orderID, req.GetUserId(), req.GetTicker(), "ORDER_REJECTED", meResp.GetRejectCode())
		return &sarvexv1.SubmitOrderResponse{RejectCode: meResp.GetRejectCode(), RejectReason: "matching rejected order"}, nil
	}

	if err := persistFillsAndOrder(ctx, s.pg, req.GetUserId(), orderID, holdResp.GetHoldId(), execReq, meResp.GetFills()); err != nil {
		return nil, err
	}
	s.publishExecEvents(orderID, req.GetUserId(), req.GetTicker(), "ORDER_ACCEPTED", "")
	for _, f := range meResp.GetFills() {
		s.publishFillEvents(f)
	}
	_ = s.runFillPosterOnce(ctx)
	if isTakeOnly(execReq) {
		_ = releaseRemainingHold(ctx, s.pg, "ioc-release:"+orderID, holdResp.GetHoldId())
	}
	out := mustGetOrder(ctx, s.pg, req.GetUserId(), orderID)
	return &sarvexv1.SubmitOrderResponse{Order: out, Fills: mapFillsForResponse(orderID, meResp.GetFills())}, nil
}

func (s *orderRouterServer) CancelOrder(ctx context.Context, req *sarvexv1.CancelOrderRequest) (*sarvexv1.CancelOrderResponse, error) {
	o, err := s.lookupOrder(ctx, req.GetUserId(), req.GetOrderId(), req.GetClientOrderId())
	if err != nil {
		return nil, err
	}
	if terminalStatus(o.GetStatus()) {
		return &sarvexv1.CancelOrderResponse{Order: o}, nil
	}
	var holdID string
	var remainingHold int64
	_ = s.pg.QueryRow(ctx, `SELECT hold_id FROM orders.orders WHERE order_id=$1`, o.GetOrderId()).Scan(&holdID)
	if holdID != "" {
		_ = s.pg.QueryRow(ctx, `SELECT GREATEST(amount_micro_usdc-committed_micro_usdc-released_micro_usdc,0) FROM ledger.holds WHERE hold_id=$1`, holdID).Scan(&remainingHold)
		if remainingHold > 0 {
			ledger := &ledgerServer{pg: s.pg}
			_, _ = ledger.ReleaseHold(ctx, &sarvexv1.ReleaseHoldRequest{
				IdempotencyKey:  "release:" + o.GetOrderId() + ":CANCEL",
				HoldId:          holdID,
				AmountMicroUsdc: remainingHold,
				ReasonCode:      "CANCEL",
			})
		}
	}
	_, err = s.pg.Exec(ctx, `UPDATE orders.orders SET status='CANCELLED', updated_at=now() WHERE order_id=$1 AND status IN ('PENDING','OPEN','PARTIAL')`, o.GetOrderId())
	if err != nil {
		return nil, mapPgErr(err)
	}
	updated, _ := s.lookupOrder(ctx, req.GetUserId(), o.GetOrderId(), "")
	s.publishExecEvents(o.GetOrderId(), o.GetUserId(), o.GetTicker(), "ORDER_CANCELLED", "")
	return &sarvexv1.CancelOrderResponse{Order: updated}, nil
}

func (s *orderRouterServer) AmendOrder(context.Context, *sarvexv1.AmendOrderRequest) (*sarvexv1.AmendOrderResponse, error) {
	return nil, status.Error(codes.Unimplemented, "amend is stubbed in milestone 8")
}

func (s *orderRouterServer) GetOrder(ctx context.Context, req *sarvexv1.GetOrderRequest) (*sarvexv1.Order, error) {
	return s.lookupOrder(ctx, req.GetUserId(), req.GetOrderId(), req.GetClientOrderId())
}

func (s *orderRouterServer) ListOrders(ctx context.Context, req *sarvexv1.ListOrdersRequest) (*sarvexv1.ListOrdersResponse, error) {
	limit := int(req.GetLimit())
	if limit <= 0 || limit > 200 {
		limit = 50
	}
	args := []any{req.GetUserId()}
	where := "user_id=$1"
	if req.GetTicker() != "" {
		args = append(args, req.GetTicker())
		where += " AND ticker=$" + strconv.Itoa(len(args))
	}
	if req.GetStatus() != sarvexv1.OrderStatus_ORDER_STATUS_UNSPECIFIED {
		args = append(args, statusDB(req.GetStatus()))
		where += " AND status=$" + strconv.Itoa(len(args)) + "::orders.order_status"
	}
	if req.GetCursor() != "" {
		args = append(args, req.GetCursor())
		where += " AND order_id > $" + strconv.Itoa(len(args))
	}
	args = append(args, limit+1)
	q := `SELECT order_id, client_order_id, user_id, ticker, side::text, action::text, price_ticks, count, filled_count, remaining_count, tif::text,
post_only, reduce_only, stp, status::text, created_at, updated_at, expires_at, hold_id, avg_fill_price_ticks
FROM orders.orders WHERE ` + where + ` ORDER BY order_id ASC LIMIT $` + strconv.Itoa(len(args))
	rows, err := s.pg.Query(ctx, q, args...)
	if err != nil {
		return nil, mapPgErr(err)
	}
	defer rows.Close()
	resp := &sarvexv1.ListOrdersResponse{}
	for rows.Next() {
		o, err := scanOrder(rows)
		if err != nil {
			return nil, err
		}
		resp.Orders = append(resp.Orders, o)
	}
	if len(resp.Orders) > limit {
		resp.NextCursor = resp.Orders[limit-1].GetOrderId()
		resp.Orders = resp.Orders[:limit]
	}
	return resp, nil
}

func (s *orderRouterServer) ListFills(ctx context.Context, req *sarvexv1.ListFillsRequest) (*sarvexv1.ListFillsResponse, error) {
	limit := int(req.GetLimit())
	if limit <= 0 || limit > 50 {
		limit = 50
	}
	from := req.GetFromGlobalSeq()
	latest := from == 0 && req.GetCursor() == "" && req.GetToGlobalSeq() == 0
	if req.GetCursor() != "" {
		if c, err := strconv.ParseUint(req.GetCursor(), 10, 64); err == nil && c > from {
			from = c
		}
	}
	args := []any{}
	where := ""
	if !latest {
		args = append(args, int64(from))
		where = "global_seq > $1"
	}
	if req.GetTicker() != "" {
		args = append(args, req.GetTicker())
		if where != "" {
			where += " AND "
		}
		where += "ticker=$" + strconv.Itoa(len(args))
	}
	if req.GetToGlobalSeq() > 0 {
		args = append(args, int64(req.GetToGlobalSeq()))
		if where != "" {
			where += " AND "
		}
		where += "global_seq <= $" + strconv.Itoa(len(args))
	}
	limitArg := limit + 1
	orderBy := "ORDER BY global_seq ASC"
	if latest {
		limitArg = limit
		orderBy = "ORDER BY global_seq DESC"
	}
	args = append(args, limitArg)
	q := `SELECT fill_id, ticker, global_seq, ticker_seq, maker_order_id, taker_order_id, maker_user_id, taker_user_id, maker_hold_id, taker_hold_id,
maker_side::text, maker_action::text, taker_side::text, taker_action::text, price_ticks, count, aggressor_side::text, maker_fee_micro_usdc, taker_fee_micro_usdc, ts
FROM orders.fills`
	if where != "" {
		q += ` WHERE ` + where
	}
	q += ` ` + orderBy + ` LIMIT $` + strconv.Itoa(len(args))
	rows, err := s.pg.Query(ctx, q, args...)
	if err != nil {
		return nil, mapPgErr(err)
	}
	defer rows.Close()
	resp := &sarvexv1.ListFillsResponse{}
	for rows.Next() {
		var fr sarvexv1.FillRecord
		var makerSide, makerAction, takerSide, takerAction, aggSide string
		var makerHoldID, takerHoldID sql.NullString
		var ts time.Time
		if err := rows.Scan(&fr.FillId, &fr.Ticker, &fr.GlobalSeq, &fr.ContractSeq, &fr.MakerOrderId, &fr.TakerOrderId, &fr.MakerUserId, &fr.TakerUserId, &makerHoldID, &takerHoldID, &makerSide, &makerAction, &takerSide, &takerAction, &fr.PriceTicks, &fr.Count, &aggSide, &fr.MakerFeeMicroUsdc, &fr.TakerFeeMicroUsdc, &ts); err != nil {
			return nil, mapPgErr(err)
		}
		fr.MakerHoldId = nullStringValue(makerHoldID)
		fr.TakerHoldId = nullStringValue(takerHoldID)
		fr.MakerSide = sideProto(makerSide)
		fr.MakerAction = actionProto(makerAction)
		fr.TakerSide = sideProto(takerSide)
		fr.TakerAction = actionProto(takerAction)
		fr.AggressorSide = sideProto(aggSide)
		fr.Ts = timestamppb.New(ts)
		resp.Fills = append(resp.Fills, &fr)
	}
	if len(resp.Fills) > limit {
		resp.NextCursor = strconv.FormatUint(resp.Fills[limit-1].GetGlobalSeq(), 10)
		resp.Fills = resp.Fills[:limit]
	}
	if latest {
		for i, j := 0, len(resp.Fills)-1; i < j; i, j = i+1, j-1 {
			resp.Fills[i], resp.Fills[j] = resp.Fills[j], resp.Fills[i]
		}
	}
	return resp, nil
}

func (s *orderRouterServer) submitToME(ctx context.Context, req *sarvexv1.SubmitOrderRequest, orderID, holdID string) (*sarvexv1.MeSubmitOrderResponse, error) {
	if s.cfg.MatchingAddr == "" {
		return nil, status.Error(codes.DeadlineExceeded, "matching unavailable")
	}
	conn, err := grpc.NewClient(s.cfg.MatchingAddr, grpc.WithTransportCredentials(insecure.NewCredentials()))
	if err != nil {
		return nil, err
	}
	defer conn.Close()
	client := sarvexv1.NewMatchingEngineClient(conn)
	cctx, cancel := context.WithTimeout(ctx, 2*time.Second)
	defer cancel()
	return client.SubmitOrder(cctx, &sarvexv1.MeSubmitOrderRequest{
		OrderId: orderID, UserId: req.GetUserId(), HoldId: holdID, Ticker: req.GetTicker(), Side: req.GetSide(), Action: req.GetAction(), PriceTicks: req.GetPriceTicks(), Count: req.GetCount(), Flags: meFlags(req), Stp: req.GetStp(),
	})
}

func executionOrderRequest(req *sarvexv1.SubmitOrderRequest, contract *sarvexv1.Contract) (*sarvexv1.SubmitOrderRequest, string, string) {
	execReq := &sarvexv1.SubmitOrderRequest{
		UserId:         req.GetUserId(),
		ClientOrderId:  req.GetClientOrderId(),
		Ticker:         req.GetTicker(),
		Side:           req.GetSide(),
		Action:         req.GetAction(),
		PriceTicks:     req.GetPriceTicks(),
		Count:          req.GetCount(),
		Tif:            req.GetTif(),
		PostOnly:       req.GetPostOnly(),
		ReduceOnly:     req.GetReduceOnly(),
		Stp:            req.GetStp(),
		ExpiresAt:      req.GetExpiresAt(),
		IdempotencyKey: req.GetIdempotencyKey(),
	}
	if execReq.GetTif() == sarvexv1.TimeInForce_TIME_IN_FORCE_UNSPECIFIED {
		execReq.Tif = sarvexv1.TimeInForce_TIME_IN_FORCE_GTC
	}
	if execReq.GetPriceTicks() != 0 {
		return execReq, "", ""
	}
	if execReq.GetPostOnly() {
		return nil, "INVALID_ORDER", "market orders cannot be post-only"
	}
	execReq.Tif = sarvexv1.TimeInForce_TIME_IN_FORCE_IOC
	if execReq.GetAction() == sarvexv1.Action_ACTION_SELL {
		execReq.PriceTicks = contract.GetMinPriceTicks()
	} else {
		execReq.PriceTicks = contract.GetMaxPriceTicks()
	}
	if execReq.GetPriceTicks() <= 0 {
		return nil, "INVALID_PRICE", "market order has no executable price range"
	}
	return execReq, "", ""
}

func meFlags(req *sarvexv1.SubmitOrderRequest) uint32 {
	var flags uint32
	switch req.GetTif() {
	case sarvexv1.TimeInForce_TIME_IN_FORCE_IOC:
		flags |= meFlagIOC
	case sarvexv1.TimeInForce_TIME_IN_FORCE_FOK:
		flags |= meFlagFOK
	}
	if req.GetPostOnly() {
		flags |= meFlagPostOnly
	}
	if req.GetReduceOnly() {
		flags |= meFlagReduceOnly
	}
	return flags
}

func isTakeOnly(req *sarvexv1.SubmitOrderRequest) bool {
	return req.GetTif() == sarvexv1.TimeInForce_TIME_IN_FORCE_IOC || req.GetTif() == sarvexv1.TimeInForce_TIME_IN_FORCE_FOK
}

func releaseRemainingHold(ctx context.Context, pool *pgxpool.Pool, idemPrefix, holdID string) error {
	if strings.TrimSpace(holdID) == "" {
		return nil
	}
	var remaining int64
	err := pool.QueryRow(ctx, `SELECT GREATEST(amount_micro_usdc - committed_micro_usdc - released_micro_usdc, 0)
FROM ledger.holds WHERE hold_id=$1`, holdID).Scan(&remaining)
	if errors.Is(err, pgx.ErrNoRows) {
		return nil
	}
	if err != nil {
		return mapPgErr(err)
	}
	if remaining <= 0 {
		return nil
	}
	ledger := &ledgerServer{pg: pool}
	_, err = ledger.ReleaseHold(ctx, &sarvexv1.ReleaseHoldRequest{
		IdempotencyKey:  idemPrefix + ":unused",
		HoldId:          holdID,
		AmountMicroUsdc: remaining,
		ReasonCode:      "IOC_UNUSED",
	})
	return err
}

func (s *orderRouterServer) validateReduceOnly(ctx context.Context, req *sarvexv1.SubmitOrderRequest) (string, string, error) {
	risk := &riskServer{pg: s.pg}
	currentPos, err := risk.currentPosition(ctx, req.GetUserId(), req.GetTicker())
	if err != nil {
		return "", "", mapPgErr(err)
	}
	delta := signedPositionDelta(req.GetSide(), req.GetAction(), req.GetCount())
	projected := currentPos + delta

	if currentPos == 0 {
		return "REDUCE_ONLY_NO_POSITION", "reduce-only order requires an open position", nil
	}
	if currentPos > 0 {
		if delta >= 0 {
			return "REDUCE_ONLY_WOULD_INCREASE", "reduce-only order would increase long exposure", nil
		}
		if projected < 0 {
			return "REDUCE_ONLY_WOULD_FLIP", "reduce-only order exceeds current long position", nil
		}
		return "", "", nil
	}
	if delta <= 0 {
		return "REDUCE_ONLY_WOULD_INCREASE", "reduce-only order would increase short exposure", nil
	}
	if projected > 0 {
		return "REDUCE_ONLY_WOULD_FLIP", "reduce-only order exceeds current short position", nil
	}
	return "", "", nil
}

func persistFillsAndOrder(ctx context.Context, pool *pgxpool.Pool, takerUserID, orderID, holdID string, req *sarvexv1.SubmitOrderRequest, fills []*sarvexv1.MeFill) error {
	tx, err := pool.BeginTx(ctx, pgx.TxOptions{})
	if err != nil {
		return mapPgErr(err)
	}
	defer tx.Rollback(ctx)

	if _, err := tx.Exec(ctx, `SELECT pg_advisory_xact_lock(9234001)`); err != nil {
		return mapPgErr(err)
	}
	var nextGlobalSeq int64
	if err := tx.QueryRow(ctx, `SELECT COALESCE(MAX(global_seq),0) FROM orders.fills`).Scan(&nextGlobalSeq); err != nil {
		return mapPgErr(err)
	}
	tickerSeqs := map[string]int64{}

	var fillQty, fillNotional int64
	for _, f := range fills {
		fillQty += f.GetCount()
		fillNotional += f.GetCount() * f.GetPriceTicks()

		nextGlobalSeq++
		gseq := nextGlobalSeq
		cseq, ok := tickerSeqs[f.GetTicker()]
		if !ok {
			if err := tx.QueryRow(ctx, `SELECT COALESCE(MAX(ticker_seq),0) FROM orders.fills WHERE ticker=$1`, f.GetTicker()).Scan(&cseq); err != nil {
				return mapPgErr(err)
			}
		}
		cseq++
		tickerSeqs[f.GetTicker()] = cseq
		f.GlobalSeq = uint64(gseq)
		f.ContractSeq = uint64(cseq)
		f.FillId = fmt.Sprintf("%s:%d:0", f.GetTicker(), cseq)

		_, err = tx.Exec(ctx, `INSERT INTO orders.fills (
fill_id, maker_order_id, taker_order_id, maker_user_id, taker_user_id, maker_hold_id, taker_hold_id, ticker, maker_side, maker_action, taker_side, taker_action, price_ticks, count, aggressor_side, maker_fee_micro_usdc, taker_fee_micro_usdc, ticker_seq, global_seq, ts
) VALUES (
$1,$2,$3,$4,$5,$6,$7,$8,$9::orders.order_side,$10::orders.order_action,$11::orders.order_side,$12::orders.order_action,$13,$14,$15::orders.order_side,$16,$17,$18,$19,$20
) ON CONFLICT (fill_id) DO NOTHING`,
			f.GetFillId(), f.GetMakerOrderId(), f.GetTakerOrderId(), f.GetMakerUserId(), f.GetTakerUserId(), f.GetMakerHoldId(), f.GetTakerHoldId(), f.GetTicker(),
			sideDB(f.GetMakerSide()), actionDB(f.GetMakerAction()), sideDB(f.GetTakerSide()), actionDB(f.GetTakerAction()),
			f.GetPriceTicks(), f.GetCount(), sideDB(f.GetAggressorSide()), f.GetMakerFeeMicroUsdc(), f.GetTakerFeeMicroUsdc(), cseq, gseq, f.GetTs().AsTime(),
		)
		if err != nil {
			return mapPgErr(err)
		}
		_, err = tx.Exec(ctx, `INSERT INTO orders.fill_posting_outbox (fill_id, global_seq, status, attempts, next_attempt_at, created_at, updated_at)
VALUES ($1,$2,'PENDING',0,now(),now(),now()) ON CONFLICT (fill_id) DO NOTHING`, f.GetFillId(), gseq)
		if err != nil {
			return mapPgErr(err)
		}
	}
	newFilled := fillQty
	newRemaining := req.GetCount() - fillQty
	if newRemaining < 0 {
		newRemaining = 0
	}
	newStatus := "OPEN"
	if newFilled > 0 && newRemaining > 0 {
		newStatus = "PARTIAL"
	}
	if newRemaining == 0 {
		newStatus = "FILLED"
	}
	if isTakeOnly(req) && newRemaining > 0 {
		newStatus = "CANCELLED"
	}
	avg := int64(0)
	if fillQty > 0 {
		avg = fillNotional / fillQty
	}
	_, err = tx.Exec(ctx, `UPDATE orders.orders
SET status=$1::orders.order_status, filled_count=$2, remaining_count=$3, avg_fill_price_ticks=$4, hold_id=COALESCE(hold_id,$5), updated_at=now()
WHERE order_id=$6`, newStatus, newFilled, newRemaining, avg, holdID, orderID)
	if err != nil {
		return mapPgErr(err)
	}
	for _, f := range fills {
		_, err = tx.Exec(ctx, `UPDATE orders.orders
SET filled_count=filled_count+$1,
remaining_count=GREATEST(remaining_count-$1,0),
status=(CASE WHEN remaining_count-$1<=0 THEN 'FILLED' ELSE 'PARTIAL' END)::orders.order_status,
updated_at=now()
WHERE order_id=$2`, f.GetCount(), f.GetMakerOrderId())
		if err != nil {
			return mapPgErr(err)
		}
	}
	if err := tx.Commit(ctx); err != nil {
		return mapPgErr(err)
	}
	return nil
}

func markRejected(ctx context.Context, pool *pgxpool.Pool, orderID, code, reason string) error {
	_, err := pool.Exec(ctx, `UPDATE orders.orders SET status='REJECTED', reject_code=$1, reject_reason=$2, updated_at=now() WHERE order_id=$3`, code, reason, orderID)
	return err
}

func mustGetOrder(ctx context.Context, pool *pgxpool.Pool, userID, orderID string) *sarvexv1.Order {
	s := &orderRouterServer{pg: pool}
	o, _ := s.lookupOrder(ctx, userID, orderID, "")
	return o
}

func (s *orderRouterServer) lookupOrder(ctx context.Context, userID, orderID, clientOrderID string) (*sarvexv1.Order, error) {
	if strings.TrimSpace(userID) == "" {
		return nil, status.Error(codes.InvalidArgument, "user_id required")
	}
	if strings.TrimSpace(orderID) == "" && strings.TrimSpace(clientOrderID) == "" {
		return nil, status.Error(codes.InvalidArgument, "order_id or client_order_id required")
	}
	q := `SELECT order_id, client_order_id, user_id, ticker, side::text, action::text, price_ticks, count, filled_count, remaining_count, tif::text,
post_only, reduce_only, stp, status::text, created_at, updated_at, expires_at, hold_id, avg_fill_price_ticks
FROM orders.orders WHERE user_id=$1 AND `
	args := []any{userID}
	if orderID != "" {
		q += "order_id=$2"
		args = append(args, orderID)
	} else {
		q += "client_order_id=$2"
		args = append(args, clientOrderID)
	}
	row := s.pg.QueryRow(ctx, q, args...)
	o, err := scanOrder(row)
	if err != nil {
		return nil, mapPgErr(err)
	}
	return o, nil
}

func scanOrder(row interface{ Scan(dest ...any) error }) (*sarvexv1.Order, error) {
	var o sarvexv1.Order
	var side, action, tif, statusStr string
	var clientOrderID, stp, holdID sql.NullString
	var created, updated time.Time
	var expires *time.Time
	if err := row.Scan(&o.OrderId, &clientOrderID, &o.UserId, &o.Ticker, &side, &action, &o.PriceTicks, &o.Count, &o.FilledCount, &o.RemainingCount, &tif, &o.PostOnly, &o.ReduceOnly, &stp, &statusStr, &created, &updated, &expires, &holdID, &o.AvgFillPriceTicks); err != nil {
		return nil, err
	}
	o.ClientOrderId = nullStringValue(clientOrderID)
	o.HoldId = nullStringValue(holdID)
	o.Side = sideProto(side)
	o.Action = actionProto(action)
	o.Tif = tifProto(tif)
	o.Stp = stpProto(nullStringValue(stp))
	o.Status = statusProto(statusStr)
	o.CreatedAt = timestamppb.New(created)
	o.UpdatedAt = timestamppb.New(updated)
	if expires != nil {
		o.ExpiresAt = timestamppb.New(*expires)
	}
	return &o, nil
}

func nullStringValue(v sql.NullString) string {
	if !v.Valid {
		return ""
	}
	return v.String
}

func mapFillsForResponse(orderID string, in []*sarvexv1.MeFill) []*sarvexv1.Fill {
	out := make([]*sarvexv1.Fill, 0, len(in))
	for _, f := range in {
		out = append(out, &sarvexv1.Fill{
			FillId:        f.GetFillId(),
			OrderId:       orderID,
			Ticker:        f.GetTicker(),
			PriceTicks:    f.GetPriceTicks(),
			Count:         f.GetCount(),
			AggressorSide: f.GetAggressorSide(),
			FeeMicroUsdc:  f.GetTakerFeeMicroUsdc(),
			Ts:            f.GetTs(),
			Seq:           f.GetGlobalSeq(),
		})
	}
	return out
}

func (s *orderRouterServer) runFillPosterOnce(ctx context.Context) error {
	rows, err := s.pg.Query(ctx, `SELECT f.fill_id, f.ticker, f.maker_hold_id, f.taker_hold_id, f.price_ticks, f.count, f.maker_side::text, f.taker_side::text
FROM orders.fill_posting_outbox o
JOIN orders.fills f ON f.fill_id=o.fill_id
WHERE o.status='PENDING' AND o.next_attempt_at <= now()
ORDER BY o.global_seq ASC
LIMIT 100`)
	if err != nil {
		return mapPgErr(err)
	}
	defer rows.Close()
	ledger := &ledgerServer{pg: s.pg}
	for rows.Next() {
		var fillID, ticker, makerHold, takerHold, makerSide, takerSide string
		var price, count int64
		if err := rows.Scan(&fillID, &ticker, &makerHold, &takerHold, &price, &count, &makerSide, &takerSide); err != nil {
			return mapPgErr(err)
		}
		amt := price * count * 10000
		if amt < 0 {
			amt = 0
		}
		dest := "LIAB:HOUSE:UNSETTLED_TRADES:" + ticker
		if makerHold != "" {
			_, _ = ledger.CommitHold(ctx, &sarvexv1.CommitHoldRequest{IdempotencyKey: "fill:" + fillID + ":maker", HoldId: makerHold, CommitAmountMicroUsdc: amt, ReleaseAmountMicroUsdc: 0, DestinationAccountCode: dest, ReasonCode: "FILL"})
		}
		if takerHold != "" {
			_, _ = ledger.CommitHold(ctx, &sarvexv1.CommitHoldRequest{IdempotencyKey: "fill:" + fillID + ":taker", HoldId: takerHold, CommitAmountMicroUsdc: amt, ReleaseAmountMicroUsdc: 0, DestinationAccountCode: dest, ReasonCode: "FILL"})
		}
		_, _ = s.pg.Exec(ctx, `UPDATE orders.fills SET ledger_post_status='POSTED' WHERE fill_id=$1`, fillID)
		_, _ = s.pg.Exec(ctx, `UPDATE orders.fill_posting_outbox SET status='POSTED', attempts=attempts+1, updated_at=now() WHERE fill_id=$1`, fillID)
	}
	return nil
}

func sideDB(v sarvexv1.Side) string {
	switch v {
	case sarvexv1.Side_SIDE_NO:
		return "NO"
	case sarvexv1.Side_SIDE_LONG:
		return "LONG"
	case sarvexv1.Side_SIDE_SHORT:
		return "SHORT"
	default:
		return "YES"
	}
}
func actionDB(v sarvexv1.Action) string {
	if v == sarvexv1.Action_ACTION_SELL {
		return "SELL"
	}
	return "BUY"
}
func tifDB(v sarvexv1.TimeInForce) string {
	switch v {
	case sarvexv1.TimeInForce_TIME_IN_FORCE_IOC:
		return "IOC"
	case sarvexv1.TimeInForce_TIME_IN_FORCE_FOK:
		return "FOK"
	default:
		return "GTC"
	}
}
func stpDB(v sarvexv1.SelfTradePreventionType) string {
	switch v {
	case sarvexv1.SelfTradePreventionType_SELF_TRADE_PREVENTION_TYPE_MAKER:
		return "MAKER"
	case sarvexv1.SelfTradePreventionType_SELF_TRADE_PREVENTION_TYPE_TAKER_AT_CROSS:
		return "TAKER_AT_CROSS"
	default:
		return ""
	}
}
func statusDB(v sarvexv1.OrderStatus) string {
	switch v {
	case sarvexv1.OrderStatus_ORDER_STATUS_PENDING:
		return "PENDING"
	case sarvexv1.OrderStatus_ORDER_STATUS_OPEN:
		return "OPEN"
	case sarvexv1.OrderStatus_ORDER_STATUS_PARTIAL:
		return "PARTIAL"
	case sarvexv1.OrderStatus_ORDER_STATUS_FILLED:
		return "FILLED"
	case sarvexv1.OrderStatus_ORDER_STATUS_CANCELLED:
		return "CANCELLED"
	case sarvexv1.OrderStatus_ORDER_STATUS_REJECTED:
		return "REJECTED"
	case sarvexv1.OrderStatus_ORDER_STATUS_EXPIRED:
		return "EXPIRED"
	default:
		return "PENDING"
	}
}
func sideProto(v string) sarvexv1.Side {
	switch v {
	case "NO":
		return sarvexv1.Side_SIDE_NO
	case "LONG":
		return sarvexv1.Side_SIDE_LONG
	case "SHORT":
		return sarvexv1.Side_SIDE_SHORT
	default:
		return sarvexv1.Side_SIDE_YES
	}
}
func actionProto(v string) sarvexv1.Action {
	if v == "SELL" {
		return sarvexv1.Action_ACTION_SELL
	}
	return sarvexv1.Action_ACTION_BUY
}
func tifProto(v string) sarvexv1.TimeInForce {
	switch v {
	case "IOC":
		return sarvexv1.TimeInForce_TIME_IN_FORCE_IOC
	case "FOK":
		return sarvexv1.TimeInForce_TIME_IN_FORCE_FOK
	default:
		return sarvexv1.TimeInForce_TIME_IN_FORCE_GTC
	}
}
func stpProto(v string) sarvexv1.SelfTradePreventionType {
	switch v {
	case "MAKER":
		return sarvexv1.SelfTradePreventionType_SELF_TRADE_PREVENTION_TYPE_MAKER
	case "TAKER_AT_CROSS":
		return sarvexv1.SelfTradePreventionType_SELF_TRADE_PREVENTION_TYPE_TAKER_AT_CROSS
	default:
		return sarvexv1.SelfTradePreventionType_SELF_TRADE_PREVENTION_TYPE_UNSPECIFIED
	}
}
func statusProto(v string) sarvexv1.OrderStatus {
	switch v {
	case "OPEN":
		return sarvexv1.OrderStatus_ORDER_STATUS_OPEN
	case "PARTIAL":
		return sarvexv1.OrderStatus_ORDER_STATUS_PARTIAL
	case "FILLED":
		return sarvexv1.OrderStatus_ORDER_STATUS_FILLED
	case "CANCELLED":
		return sarvexv1.OrderStatus_ORDER_STATUS_CANCELLED
	case "REJECTED":
		return sarvexv1.OrderStatus_ORDER_STATUS_REJECTED
	case "EXPIRED":
		return sarvexv1.OrderStatus_ORDER_STATUS_EXPIRED
	default:
		return sarvexv1.OrderStatus_ORDER_STATUS_PENDING
	}
}
func terminalStatus(v sarvexv1.OrderStatus) bool {
	return v == sarvexv1.OrderStatus_ORDER_STATUS_FILLED || v == sarvexv1.OrderStatus_ORDER_STATUS_CANCELLED || v == sarvexv1.OrderStatus_ORDER_STATUS_REJECTED || v == sarvexv1.OrderStatus_ORDER_STATUS_EXPIRED
}

func (s *orderRouterServer) publishExecEvents(orderID, userID, ticker, eventType, rejectCode string) {
	if s.nc == nil {
		return
	}
	msg := map[string]any{
		"order_id": orderID, "user_id": userID, "ticker": ticker, "event_type": eventType, "reject_code": rejectCode, "ts": time.Now().UTC().Format(time.RFC3339Nano),
	}
	b, _ := json.Marshal(msg)
	_ = s.nc.Publish("exec.events", b)
	userView := map[string]any{"order_id": orderID, "ticker": ticker, "event_type": eventType, "ts": msg["ts"]}
	ub, _ := json.Marshal(userView)
	_ = s.nc.Publish("exec.user."+userID, ub)
}

func (s *orderRouterServer) publishFillEvents(f *sarvexv1.MeFill) {
	if s.nc == nil || f == nil {
		return
	}
	b, _ := json.Marshal(f)
	_ = s.nc.Publish("exec.fills."+f.GetTicker(), b)
	_ = s.nc.Publish("md.trade."+f.GetTicker(), b)
	_ = s.nc.Publish("md.ticker."+f.GetTicker(), b)
	maker := map[string]any{"fill_id": f.GetFillId(), "ticker": f.GetTicker(), "price_ticks": f.GetPriceTicks(), "count": f.GetCount(), "role": "maker", "ts": time.Now().UTC().Format(time.RFC3339Nano)}
	taker := map[string]any{"fill_id": f.GetFillId(), "ticker": f.GetTicker(), "price_ticks": f.GetPriceTicks(), "count": f.GetCount(), "role": "taker", "ts": time.Now().UTC().Format(time.RFC3339Nano)}
	mb, _ := json.Marshal(maker)
	tb, _ := json.Marshal(taker)
	_ = s.nc.Publish("exec.fills.user."+f.GetMakerUserId(), mb)
	_ = s.nc.Publish("exec.fills.user."+f.GetTakerUserId(), tb)
}
