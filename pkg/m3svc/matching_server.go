package m3svc

import (
	"context"
	"sort"
	"time"

	sarvexv1 "github.com/sarvex/proto/gen/go/sarvex/v1"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
	"google.golang.org/protobuf/types/known/emptypb"
	"google.golang.org/protobuf/types/known/timestamppb"
)

type demoBook struct {
	ticker      string
	contractSeq uint64
	closed      bool
	orders      map[string]*demoMEOrder
}

type demoMEOrder struct {
	orderID    string
	userID     string
	holdID     string
	ticker     string
	side       sarvexv1.Side
	action     sarvexv1.Action
	priceTicks int64
	openQty    int64
}

func newMatchingEngineServer() *matchingEngineServer {
	return &matchingEngineServer{books: map[string]*demoBook{}, global: uint64(time.Now().UnixNano())}
}

func (s *matchingEngineServer) AddBook(ctx context.Context, req *sarvexv1.AddBookRequest) (*emptypb.Empty, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	if b, ok := s.books[req.GetTicker()]; ok {
		b.closed = false
		b.orders = map[string]*demoMEOrder{}
		return &emptypb.Empty{}, nil
	}
	s.ensureBook(req.GetTicker())
	return &emptypb.Empty{}, nil
}

func (s *matchingEngineServer) CloseBook(ctx context.Context, req *sarvexv1.CloseBookRequest) (*sarvexv1.CloseBookResponse, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	b := s.ensureBook(req.GetTicker())
	if !b.closed {
		s.global++
		b.contractSeq++
		b.closed = true
	}
	return &sarvexv1.CloseBookResponse{Ticker: req.GetTicker(), CloseGlobalSeq: s.global, CloseContractSeq: b.contractSeq}, nil
}

func (s *matchingEngineServer) SubmitOrder(ctx context.Context, req *sarvexv1.MeSubmitOrderRequest) (*sarvexv1.MeSubmitOrderResponse, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	if req.GetTicker() == "" || req.GetOrderId() == "" || req.GetCount() <= 0 || req.GetPriceTicks() <= 0 {
		return nil, status.Error(codes.InvalidArgument, "ticker, order_id, count and price_ticks are required")
	}
	b := s.ensureBook(req.GetTicker())
	if b.closed {
		return &sarvexv1.MeSubmitOrderResponse{OrderId: req.GetOrderId(), RejectCode: "BOOK_CLOSED"}, nil
	}
	if _, ok := b.orders[req.GetOrderId()]; ok {
		return &sarvexv1.MeSubmitOrderResponse{OrderId: req.GetOrderId(), Accepted: true, GlobalSeq: s.global, ContractSeq: b.contractSeq}, nil
	}

	incoming := &demoMEOrder{
		orderID: req.GetOrderId(), userID: req.GetUserId(), holdID: req.GetHoldId(), ticker: req.GetTicker(),
		side: req.GetSide(), action: req.GetAction(), priceTicks: req.GetPriceTicks(), openQty: req.GetCount(),
	}
	makers := make([]*demoMEOrder, 0, len(b.orders))
	for _, o := range b.orders {
		if crosses(incoming, o) {
			makers = append(makers, o)
		}
	}
	if req.GetFlags()&meFlagPostOnly != 0 && len(makers) > 0 {
		return &sarvexv1.MeSubmitOrderResponse{OrderId: req.GetOrderId(), RejectCode: "POST_ONLY_WOULD_MATCH"}, nil
	}
	sort.Slice(makers, func(i, j int) bool {
		if makers[i].priceTicks == makers[j].priceTicks {
			return makers[i].orderID < makers[j].orderID
		}
		if incoming.action == sarvexv1.Action_ACTION_BUY {
			return makers[i].priceTicks < makers[j].priceTicks
		}
		return makers[i].priceTicks > makers[j].priceTicks
	})

	resp := &sarvexv1.MeSubmitOrderResponse{OrderId: req.GetOrderId(), Accepted: true}
	if req.GetFlags()&meFlagFOK != 0 {
		var available int64
		for _, maker := range makers {
			available += maker.openQty
			if available >= incoming.openQty {
				break
			}
		}
		if available < incoming.openQty {
			return &sarvexv1.MeSubmitOrderResponse{OrderId: req.GetOrderId(), RejectCode: "FOK_NOT_FILLED"}, nil
		}
	}
	for _, maker := range makers {
		if incoming.openQty <= 0 {
			break
		}
		qty := incoming.openQty
		if maker.openQty < qty {
			qty = maker.openQty
		}
		if qty <= 0 {
			continue
		}
		s.global++
		b.contractSeq++
		maker.openQty -= qty
		incoming.openQty -= qty
		fillID := req.GetTicker() + ":" + utoa(b.contractSeq) + ":0"
		resp.Fills = append(resp.Fills, &sarvexv1.MeFill{
			FillId:        fillID,
			MakerOrderId:  maker.orderID,
			TakerOrderId:  incoming.orderID,
			MakerUserId:   maker.userID,
			TakerUserId:   incoming.userID,
			PriceTicks:    maker.priceTicks,
			Count:         qty,
			AggressorSide: incoming.side,
			Ticker:        req.GetTicker(),
			GlobalSeq:     s.global,
			ContractSeq:   b.contractSeq,
			Ts:            timestamppb.Now(),
			MakerHoldId:   maker.holdID,
			TakerHoldId:   incoming.holdID,
			MakerSide:     maker.side,
			MakerAction:   maker.action,
			TakerSide:     incoming.side,
			TakerAction:   incoming.action,
		})
		if maker.openQty == 0 {
			delete(b.orders, maker.orderID)
		}
	}
	if incoming.openQty > 0 && req.GetFlags()&meFlagIOC == 0 && req.GetFlags()&meFlagFOK == 0 {
		b.orders[incoming.orderID] = incoming
	}
	if len(resp.Fills) == 0 {
		s.global++
		b.contractSeq++
	}
	resp.GlobalSeq = s.global
	resp.ContractSeq = b.contractSeq
	return resp, nil
}

func (s *matchingEngineServer) CancelOrder(ctx context.Context, req *sarvexv1.MeCancelOrderRequest) (*sarvexv1.MeCancelOrderResponse, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	for _, b := range s.books {
		if o, ok := b.orders[req.GetOrderId()]; ok {
			delete(b.orders, req.GetOrderId())
			s.global++
			b.contractSeq++
			return &sarvexv1.MeCancelOrderResponse{OrderId: req.GetOrderId(), Cancelled: true, CancelledQty: o.openQty}, nil
		}
	}
	return &sarvexv1.MeCancelOrderResponse{OrderId: req.GetOrderId(), RejectCode: "ORDER_NOT_FOUND"}, nil
}

func (s *matchingEngineServer) AmendOrder(ctx context.Context, req *sarvexv1.MeAmendOrderRequest) (*sarvexv1.MeAmendOrderResponse, error) {
	return &sarvexv1.MeAmendOrderResponse{OrderId: req.GetOrderId(), RejectCode: "AMEND_NOT_SUPPORTED"}, nil
}

func (s *matchingEngineServer) GetBookSnapshot(ctx context.Context, req *sarvexv1.GetBookSnapshotRequest) (*sarvexv1.BookSnapshot, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	b := s.ensureBook(req.GetTicker())
	depth := int(req.GetDepth())
	if depth <= 0 {
		depth = 25
	}
	bids := map[int64]*sarvexv1.PriceLevel{}
	asks := map[int64]*sarvexv1.PriceLevel{}
	for _, o := range b.orders {
		levels := bids
		if o.action == sarvexv1.Action_ACTION_SELL {
			levels = asks
		}
		lvl := levels[o.priceTicks]
		if lvl == nil {
			lvl = &sarvexv1.PriceLevel{PriceTicks: o.priceTicks}
			levels[o.priceTicks] = lvl
		}
		lvl.TotalQty += o.openQty
		lvl.OrderCount++
	}
	resp := &sarvexv1.BookSnapshot{Ticker: req.GetTicker(), Seq: b.contractSeq, Ts: timestamppb.Now()}
	resp.Bids = priceLevels(bids, depth, true)
	resp.Asks = priceLevels(asks, depth, false)
	return resp, nil
}

func (s *matchingEngineServer) StreamExecutions(req *sarvexv1.StreamExecutionsRequest, stream sarvexv1.MatchingEngine_StreamExecutionsServer) error {
	return status.Error(codes.Unimplemented, "stream executions is not implemented in MVP matching server")
}

func (s *matchingEngineServer) ensureBook(ticker string) *demoBook {
	if b, ok := s.books[ticker]; ok {
		return b
	}
	b := &demoBook{ticker: ticker, orders: map[string]*demoMEOrder{}}
	s.books[ticker] = b
	return b
}

func crosses(taker, maker *demoMEOrder) bool {
	if taker.ticker != maker.ticker || taker.side != maker.side || taker.action == maker.action {
		return false
	}
	if taker.action == sarvexv1.Action_ACTION_BUY {
		return taker.priceTicks >= maker.priceTicks
	}
	return taker.priceTicks <= maker.priceTicks
}

func priceLevels(src map[int64]*sarvexv1.PriceLevel, depth int, desc bool) []*sarvexv1.PriceLevel {
	prices := make([]int64, 0, len(src))
	for p := range src {
		prices = append(prices, p)
	}
	sort.Slice(prices, func(i, j int) bool {
		if desc {
			return prices[i] > prices[j]
		}
		return prices[i] < prices[j]
	})
	if len(prices) > depth {
		prices = prices[:depth]
	}
	out := make([]*sarvexv1.PriceLevel, 0, len(prices))
	for _, p := range prices {
		out = append(out, src[p])
	}
	return out
}

func utoa(v uint64) string {
	if v == 0 {
		return "0"
	}
	buf := [20]byte{}
	i := len(buf)
	for v > 0 {
		i--
		buf[i] = byte('0' + v%10)
		v /= 10
	}
	return string(buf[i:])
}
