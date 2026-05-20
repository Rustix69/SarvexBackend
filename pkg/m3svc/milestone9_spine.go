package m3svc

import (
	"context"
	"encoding/json"
	"strings"
	"time"

	"github.com/jackc/pgx/v5/pgxpool"
	"github.com/nats-io/nats.go"
	sarvexv1 "github.com/sarvex/proto/gen/go/sarvex/v1"
)

func runLedgerOutboxPublisher(ctx context.Context, pg *pgxpool.Pool, nc *nats.Conn) {
	t := time.NewTicker(500 * time.Millisecond)
	defer t.Stop()
	for {
		select {
		case <-ctx.Done():
			return
		case <-t.C:
			rows, err := pg.Query(ctx, `SELECT tx_id, payload FROM ledger.ledger_event_outbox WHERE status='PENDING' AND next_attempt_at <= now() ORDER BY outbox_id ASC LIMIT 100`)
			if err != nil {
				continue
			}
			for rows.Next() {
				var txID int64
				var payload []byte
				if err := rows.Scan(&txID, &payload); err != nil {
					continue
				}
				_ = nc.Publish("ledger.events", payload)
				urows, err := pg.Query(ctx, `SELECT DISTINCT a.user_id
FROM ledger.accounts a
JOIN ledger.entries e ON e.account_id=a.account_id
WHERE e.tx_id=$1 AND a.user_id IS NOT NULL`, txID)
				if err == nil {
					for urows.Next() {
						var userID string
						if err := urows.Scan(&userID); err == nil && userID != "" {
							bal := map[string]any{"user_id": userID, "tx_id": txID, "ts": time.Now().UTC().Format(time.RFC3339Nano)}
							b, _ := json.Marshal(bal)
							_ = nc.Publish("ledger.balance.user."+userID, b)
						}
					}
					urows.Close()
				}
				_, _ = pg.Exec(ctx, `UPDATE ledger.ledger_event_outbox SET status='POSTED', published_at=now(), updated_at=now() WHERE tx_id=$1`, txID)
			}
			rows.Close()
		}
	}
}

func runPositionFillConsumer(ctx context.Context, pg *pgxpool.Pool, nc *nats.Conn) {
	_, _ = nc.Subscribe("exec.fills.*", func(msg *nats.Msg) {
		var f sarvexv1.MeFill
		if err := json.Unmarshal(msg.Data, &f); err != nil {
			return
		}
		applyPositionFill(ctx, pg, &f)
	})
	<-ctx.Done()
}

func applyPositionFill(ctx context.Context, pg *pgxpool.Pool, f *sarvexv1.MeFill) {
	tx, err := pg.Begin(ctx)
	if err != nil {
		return
	}
	defer tx.Rollback(ctx)
	var last int64
	_ = tx.QueryRow(ctx, `INSERT INTO position.consumer_offsets(stream_name,last_global_seq,updated_at) VALUES ('exec.fills',0,now()) ON CONFLICT (stream_name) DO NOTHING RETURNING last_global_seq`).Scan(&last)
	_ = tx.QueryRow(ctx, `SELECT last_global_seq FROM position.consumer_offsets WHERE stream_name='exec.fills' FOR UPDATE`).Scan(&last)
	g := int64(f.GetGlobalSeq())
	if g > last+1 {
		replayMissingRange(ctx, pg, uint64(last+1), uint64(g-1), f.GetTicker())
	}
	var exists string
	err = tx.QueryRow(ctx, `SELECT fill_id FROM position.applied_fills WHERE fill_id=$1`, f.GetFillId()).Scan(&exists)
	if err == nil {
		_, _ = tx.Exec(ctx, `UPDATE position.consumer_offsets SET last_global_seq=GREATEST(last_global_seq,$1), updated_at=now() WHERE stream_name='exec.fills'`, g)
		_ = tx.Commit(ctx)
		return
	}
	applyLeg := func(userID, side string, qty int64) {
		sign := int64(1)
		if side == "NO" || side == "SHORT" {
			sign = -1
		}
		delta := qty * sign
		var before int64
		_ = tx.QueryRow(ctx, `SELECT COALESCE(net_qty,0) FROM position.positions WHERE user_id=$1 AND ticker=$2`, userID, f.GetTicker()).Scan(&before)
		_, _ = tx.Exec(ctx, `INSERT INTO position.positions (user_id,ticker,net_qty,avg_cost_micro_usdc,realized_pnl_micro_usdc,last_global_seq,updated_at)
VALUES ($1,$2,$3,$4,0,$5,now())
ON CONFLICT (user_id,ticker) DO UPDATE SET net_qty=position.positions.net_qty+$3,last_global_seq=GREATEST(position.positions.last_global_seq,$5),updated_at=now()`,
			userID, f.GetTicker(), delta, f.GetPriceTicks()*10000, g)
		_, _ = tx.Exec(ctx, `INSERT INTO position.position_history (user_id,ticker,net_qty_before,net_qty_after,fill_id,global_seq,ts) VALUES ($1,$2,$3,$4,$5,$6,now())`,
			userID, f.GetTicker(), before, before+delta, f.GetFillId(), g)
	}
	applyLeg(f.GetMakerUserId(), strings.TrimPrefix(f.GetMakerSide().String(), "SIDE_"), f.GetCount())
	applyLeg(f.GetTakerUserId(), strings.TrimPrefix(f.GetTakerSide().String(), "SIDE_"), f.GetCount())
	_, _ = tx.Exec(ctx, `INSERT INTO position.applied_fills (fill_id,ticker,global_seq,applied_at) VALUES ($1,$2,$3,now()) ON CONFLICT (fill_id) DO NOTHING`,
		f.GetFillId(), f.GetTicker(), g)
	_, _ = tx.Exec(ctx, `UPDATE position.consumer_offsets SET last_global_seq=GREATEST(last_global_seq,$1), updated_at=now() WHERE stream_name='exec.fills'`, g)
	_ = tx.Commit(ctx)
}

func replayMissingRange(ctx context.Context, pg *pgxpool.Pool, from, to uint64, ticker string) {
	s := &orderRouterServer{pg: pg}
	resp, err := s.ListFills(ctx, &sarvexv1.ListFillsRequest{Ticker: ticker, FromGlobalSeq: from - 1, ToGlobalSeq: to, Limit: int32(to - from + 1)})
	if err != nil {
		return
	}
	for _, fr := range resp.GetFills() {
		f := &sarvexv1.MeFill{
			FillId:       fr.GetFillId(),
			MakerUserId:  fr.GetMakerUserId(),
			TakerUserId:  fr.GetTakerUserId(),
			Ticker:       fr.GetTicker(),
			PriceTicks:   fr.GetPriceTicks(),
			Count:        fr.GetCount(),
			MakerSide:    fr.GetMakerSide(),
			TakerSide:    fr.GetTakerSide(),
			GlobalSeq:    fr.GetGlobalSeq(),
			MakerOrderId: fr.GetMakerOrderId(),
			TakerOrderId: fr.GetTakerOrderId(),
		}
		applyPositionFill(ctx, pg, f)
	}
}

func runRiskFillConsumer(ctx context.Context, pg *pgxpool.Pool, nc *nats.Conn) {
	_, _ = nc.Subscribe("exec.fills.*", func(msg *nats.Msg) {
		var f sarvexv1.MeFill
		if err := json.Unmarshal(msg.Data, &f); err != nil {
			return
		}
		_, _ = pg.Exec(ctx, `INSERT INTO risk.working_orders_summary (user_id,ticker,side,total_qty,total_max_loss_micro_usdc,updated_at)
VALUES ($1,$2,$3,0,0,now()) ON CONFLICT (user_id,ticker,side) DO UPDATE SET total_qty=GREATEST(risk.working_orders_summary.total_qty-$4,0),updated_at=now()`,
			f.GetMakerUserId(), f.GetTicker(), strings.TrimPrefix(f.GetMakerSide().String(), "SIDE_"), f.GetCount())
		_, _ = pg.Exec(ctx, `INSERT INTO risk.working_orders_summary (user_id,ticker,side,total_qty,total_max_loss_micro_usdc,updated_at)
VALUES ($1,$2,$3,0,0,now()) ON CONFLICT (user_id,ticker,side) DO UPDATE SET total_qty=GREATEST(risk.working_orders_summary.total_qty-$4,0),updated_at=now()`,
			f.GetTakerUserId(), f.GetTicker(), strings.TrimPrefix(f.GetTakerSide().String(), "SIDE_"), f.GetCount())
	})
	<-ctx.Done()
}

func runAuditConsumer(ctx context.Context, pg *pgxpool.Pool, nc *nats.Conn) {
	write := func(subject string, data []byte) {
		var seq int64
		if err := pg.QueryRow(ctx, `SELECT nextval('audit.event_seq_gen')`).Scan(&seq); err != nil {
			return
		}
		_, _ = pg.Exec(ctx, `INSERT INTO audit.events (event_seq,service,type,actor,subject,payload,ts) VALUES ($1,$2,$3,$4,$5,$6,now())`,
			seq, "nats-consumer", subject, "system", "", data)
	}
	_, _ = nc.Subscribe("exec.events", func(msg *nats.Msg) { write("exec.events", msg.Data) })
	_, _ = nc.Subscribe("ledger.events", func(msg *nats.Msg) { write("ledger.events", msg.Data) })
	<-ctx.Done()
}
