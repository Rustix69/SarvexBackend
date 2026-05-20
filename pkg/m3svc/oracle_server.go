package m3svc

import (
	"context"
	"errors"
	"strings"
	"time"

	"github.com/jackc/pgx/v5"
	sarvexv1 "github.com/sarvex/proto/gen/go/sarvex/v1"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
	"google.golang.org/protobuf/types/known/timestamppb"
)

func (s *oracleServer) ProposeResolution(ctx context.Context, req *sarvexv1.ProposeResolutionRequest) (*sarvexv1.Resolution, error) {
	if strings.TrimSpace(req.GetEventTicker()) == "" {
		return nil, status.Error(codes.InvalidArgument, "event_ticker required")
	}
	_, err := s.pg.Exec(ctx, `INSERT INTO oracle.attestations
(event_ticker, attestor_id, source, numeric_value, categorical_value, signature, observed_at, received_at)
VALUES ($1,$2,$3,NULLIF($4,0),NULLIF($5,''),$6,now(),now())
ON CONFLICT (event_ticker, attestor_id, source) DO UPDATE SET
numeric_value=EXCLUDED.numeric_value, categorical_value=EXCLUDED.categorical_value, signature=EXCLUDED.signature, observed_at=now()`,
		req.GetEventTicker(), defaultStr(req.GetAttestorId(), "admin"), defaultStr(req.GetSource(), "manual"), req.GetNumericValue(), req.GetCategoricalValue(), req.GetSignature(),
	)
	if err != nil {
		return nil, mapPgErr(err)
	}
	_, err = s.pg.Exec(ctx, `INSERT INTO oracle.resolutions (event_ticker, status, numeric_value, categorical_value, proposed_at, attestor_count, required_quorum)
VALUES ($1,'PROPOSED',NULLIF($2,0),NULLIF($3,''),now(),1,1)
ON CONFLICT (event_ticker) DO UPDATE SET status='PROPOSED', numeric_value=EXCLUDED.numeric_value, categorical_value=EXCLUDED.categorical_value, proposed_at=now(), attestor_count=GREATEST(oracle.resolutions.attestor_count,1)`,
		req.GetEventTicker(), req.GetNumericValue(), req.GetCategoricalValue(),
	)
	if err != nil {
		return nil, mapPgErr(err)
	}
	return s.GetResolution(ctx, &sarvexv1.GetResolutionRequest{EventTicker: req.GetEventTicker()})
}

func (s *oracleServer) FinalizeResolution(ctx context.Context, req *sarvexv1.FinalizeResolutionRequest) (*sarvexv1.Resolution, error) {
	if strings.TrimSpace(req.GetEventTicker()) == "" {
		return nil, status.Error(codes.InvalidArgument, "event_ticker required")
	}
	tx, err := s.pg.BeginTx(ctx, pgx.TxOptions{})
	if err != nil {
		return nil, mapPgErr(err)
	}
	defer tx.Rollback(ctx)

	_, err = tx.Exec(ctx, `INSERT INTO oracle.resolutions (event_ticker, status, finalized_at)
VALUES ($1,'PENDING',NULL)
ON CONFLICT (event_ticker) DO NOTHING`, req.GetEventTicker())
	if err != nil {
		return nil, mapPgErr(err)
	}
	var statusStr string
	err = tx.QueryRow(ctx, `SELECT status::text FROM oracle.resolutions WHERE event_ticker=$1 FOR UPDATE`, req.GetEventTicker()).Scan(&statusStr)
	if err != nil {
		return nil, mapPgErr(err)
	}
	if statusStr != "FINALIZED" {
		_, err = tx.Exec(ctx, `UPDATE oracle.resolutions SET status='FINALIZED', finalized_at=now() WHERE event_ticker=$1`, req.GetEventTicker())
		if err != nil {
			return nil, mapPgErr(err)
		}
	}
	if err := tx.Commit(ctx); err != nil {
		return nil, mapPgErr(err)
	}
	res, err := s.GetResolution(ctx, &sarvexv1.GetResolutionRequest{EventTicker: req.GetEventTicker()})
	if err == nil && s.nc != nil {
		b, _ := structToJSON(map[string]any{
			"event_ticker":      res.GetEventTicker(),
			"numeric_value":     res.GetNumericValue(),
			"categorical_value": res.GetCategoricalValue(),
			"finalized_at":      time.Now().UTC().Format(time.RFC3339Nano),
		})
		_ = s.nc.Publish("oracle.resolutions.finalized."+req.GetEventTicker(), b)
	}
	return res, err
}

func (s *oracleServer) GetResolution(ctx context.Context, req *sarvexv1.GetResolutionRequest) (*sarvexv1.Resolution, error) {
	if strings.TrimSpace(req.GetEventTicker()) == "" {
		return nil, status.Error(codes.InvalidArgument, "event_ticker required")
	}
	var r sarvexv1.Resolution
	var statusStr string
	var proposedAt, finalizedAt *time.Time
	err := s.pg.QueryRow(ctx, `SELECT event_ticker, COALESCE(numeric_value,0), COALESCE(categorical_value,''), status::text, proposed_at, finalized_at
FROM oracle.resolutions WHERE event_ticker=$1`, req.GetEventTicker()).
		Scan(&r.EventTicker, &r.NumericValue, &r.CategoricalValue, &statusStr, &proposedAt, &finalizedAt)
	if errors.Is(err, pgx.ErrNoRows) {
		return nil, status.Error(codes.NotFound, "resolution not found")
	}
	if err != nil {
		return nil, mapPgErr(err)
	}
	rows, err := s.pg.Query(ctx, `SELECT attestor_id, source, COALESCE(numeric_value,0), COALESCE(categorical_value,''), signature, observed_at
FROM oracle.attestations WHERE event_ticker=$1 ORDER BY received_at ASC`, req.GetEventTicker())
	if err == nil {
		defer rows.Close()
		for rows.Next() {
			a := &sarvexv1.Attestation{}
			var obs time.Time
			if err := rows.Scan(&a.AttestorId, &a.Source, &a.NumericValue, &a.CategoricalValue, &a.Signature, &obs); err == nil {
				a.ObservedAt = timestamppb.New(obs)
				r.Attestations = append(r.Attestations, a)
			}
		}
	}
	r.Status = resolutionStatusProto(statusStr)
	if proposedAt != nil {
		r.ProposedAt = timestamppb.New(*proposedAt)
	}
	if finalizedAt != nil {
		r.FinalizedAt = timestamppb.New(*finalizedAt)
	}
	return &r, nil
}

func (s *oracleServer) AdminForceResolution(ctx context.Context, req *sarvexv1.AdminForceResolutionRequest) (*sarvexv1.Resolution, error) {
	_, err := s.ProposeResolution(ctx, &sarvexv1.ProposeResolutionRequest{
		EventTicker:      req.GetEventTicker(),
		NumericValue:     req.GetNumericValue(),
		CategoricalValue: req.GetCategoricalValue(),
		Source:           "admin_force",
		AttestorId:       defaultStr(req.GetAdminUserId(), "admin"),
	})
	if err != nil {
		return nil, err
	}
	return s.FinalizeResolution(ctx, &sarvexv1.FinalizeResolutionRequest{EventTicker: req.GetEventTicker()})
}

func resolutionStatusProto(v string) sarvexv1.ResolutionStatus {
	switch v {
	case "PENDING":
		return sarvexv1.ResolutionStatus_RESOLUTION_STATUS_PENDING
	case "PROPOSED":
		return sarvexv1.ResolutionStatus_RESOLUTION_STATUS_PROPOSED
	case "FINALIZED":
		return sarvexv1.ResolutionStatus_RESOLUTION_STATUS_FINALIZED
	case "DISPUTED":
		return sarvexv1.ResolutionStatus_RESOLUTION_STATUS_DISPUTED
	default:
		return sarvexv1.ResolutionStatus_RESOLUTION_STATUS_UNSPECIFIED
	}
}
