package m3svc

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"sort"
	"strings"
	"time"

	"github.com/jackc/pgx/v5"
	"github.com/jackc/pgx/v5/pgconn"
	sarvexv1 "github.com/sarvex/proto/gen/go/sarvex/v1"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
	"google.golang.org/protobuf/types/known/emptypb"
	"google.golang.org/protobuf/types/known/timestamppb"
)

type entrySpec struct {
	accountCode string
	direction   string
	amount      int64
	memo        string
}

type accountState struct {
	accountID int64
	balance   int64
	nextSeq   int64
}

func (s *ledgerServer) PostTransaction(ctx context.Context, req *sarvexv1.PostTransactionRequest) (*sarvexv1.PostTransactionResponse, error) {
	if strings.TrimSpace(req.GetIdempotencyKey()) == "" {
		return nil, status.Error(codes.InvalidArgument, "idempotency_key is required")
	}
	if len(req.GetEntries()) == 0 {
		return nil, status.Error(codes.InvalidArgument, "entries are required")
	}

	entries := make([]entrySpec, 0, len(req.GetEntries()))
	for _, e := range req.GetEntries() {
		if e.GetAmountMicroUsdc() <= 0 {
			return nil, status.Error(codes.InvalidArgument, "entry amount must be positive")
		}
		dir := strings.ToUpper(strings.TrimSpace(e.GetDirection()))
		if dir != "DR" && dir != "CR" {
			return nil, status.Error(codes.InvalidArgument, "entry direction must be DR or CR")
		}
		entries = append(entries, entrySpec{
			accountCode: strings.TrimSpace(e.GetAccountCode()),
			direction:   dir,
			amount:      e.GetAmountMicroUsdc(),
			memo:        e.GetMemo(),
		})
	}

	metaJSON, err := structToJSON(req.GetMetadata())
	if err != nil {
		return nil, status.Errorf(codes.InvalidArgument, "invalid metadata: %v", err)
	}

	tx, err := s.pg.BeginTx(ctx, pgx.TxOptions{})
	if err != nil {
		return nil, status.Errorf(codes.Internal, "begin tx: %v", err)
	}
	defer tx.Rollback(ctx)

	txID, postedAt, existed, err := createOrGetTransaction(ctx, tx, req.GetIdempotencyKey(), req.GetReasonCode(), metaJSON)
	if err != nil {
		return nil, mapPgErr(err)
	}
	if !existed {
		if err := applyEntries(ctx, tx, entries); err != nil {
			return nil, err
		}
		if err := insertOutbox(ctx, tx, txID, "LEDGER_TRANSACTION_POSTED", metaJSON); err != nil {
			return nil, mapPgErr(err)
		}
	}

	if err := tx.Commit(ctx); err != nil {
		return nil, mapPgErr(err)
	}

	return &sarvexv1.PostTransactionResponse{TxId: fmt.Sprintf("%d", txID), PostedAt: timestamppb.New(postedAt)}, nil
}

func (s *ledgerServer) PlaceHold(ctx context.Context, req *sarvexv1.PlaceHoldRequest) (*sarvexv1.PlaceHoldResponse, error) {
	if strings.TrimSpace(req.GetIdempotencyKey()) == "" || strings.TrimSpace(req.GetUserId()) == "" {
		return nil, status.Error(codes.InvalidArgument, "idempotency_key and user_id are required")
	}
	if req.GetAmountMicroUsdc() <= 0 {
		return nil, status.Error(codes.InvalidArgument, "amount must be positive")
	}

	tx, err := s.pg.BeginTx(ctx, pgx.TxOptions{})
	if err != nil {
		return nil, status.Errorf(codes.Internal, "begin tx: %v", err)
	}
	defer tx.Rollback(ctx)

	var existingHold string
	err = tx.QueryRow(ctx,
		`SELECT hold_id FROM ledger.hold_operations WHERE idempotency_key=$1`,
		req.GetIdempotencyKey(),
	).Scan(&existingHold)
	if err == nil {
		if err := tx.Commit(ctx); err != nil {
			return nil, mapPgErr(err)
		}
		return &sarvexv1.PlaceHoldResponse{HoldId: existingHold}, nil
	}
	if !errors.Is(err, pgx.ErrNoRows) {
		return nil, mapPgErr(err)
	}

	cashCode := fmt.Sprintf("LIAB:USER:%s:CASH", req.GetUserId())
	holdsCode := fmt.Sprintf("LIAB:USER:%s:HOLDS", req.GetUserId())
	if _, err := ensureAccount(ctx, tx, cashCode, "LIABILITY", "USDC", req.GetUserId()); err != nil {
		return nil, mapPgErr(err)
	}
	if _, err := ensureAccount(ctx, tx, holdsCode, "LIABILITY", "USDC", req.GetUserId()); err != nil {
		return nil, mapPgErr(err)
	}

	holdID := fmt.Sprintf("hold_%d", time.Now().UnixNano())
	_, err = tx.Exec(ctx,
		`INSERT INTO ledger.holds (hold_id, user_id, amount_micro_usdc, reason) VALUES ($1,$2,$3,$4)`,
		holdID, req.GetUserId(), req.GetAmountMicroUsdc(), req.GetReason(),
	)
	if err != nil {
		return nil, mapPgErr(err)
	}

	reasonCode := "HOLD_PLACE"
	meta := json.RawMessage(`{"op":"PLACE_HOLD"}`)
	txID, _, _, err := createOrGetTransaction(ctx, tx, req.GetIdempotencyKey(), reasonCode, meta)
	if err != nil {
		return nil, mapPgErr(err)
	}

	if err := applyEntries(ctx, tx, []entrySpec{
		{accountCode: cashCode, direction: "DR", amount: req.GetAmountMicroUsdc(), memo: "place hold"},
		{accountCode: holdsCode, direction: "CR", amount: req.GetAmountMicroUsdc(), memo: "place hold"},
	}); err != nil {
		return nil, err
	}

	_, err = tx.Exec(ctx,
		`INSERT INTO ledger.hold_operations (idempotency_key, hold_id, operation_type, amount_micro_usdc, ledger_tx_id)
VALUES ($1,$2,'PLACE',$3,$4)`,
		req.GetIdempotencyKey(), holdID, req.GetAmountMicroUsdc(), txID,
	)
	if err != nil {
		return nil, mapPgErr(err)
	}

	if err := insertOutbox(ctx, tx, txID, "LEDGER_TRANSACTION_POSTED", meta); err != nil {
		return nil, mapPgErr(err)
	}

	if err := tx.Commit(ctx); err != nil {
		return nil, mapPgErr(err)
	}
	return &sarvexv1.PlaceHoldResponse{HoldId: holdID}, nil
}

func (s *ledgerServer) ReleaseHold(ctx context.Context, req *sarvexv1.ReleaseHoldRequest) (*emptypb.Empty, error) {
	if strings.TrimSpace(req.GetIdempotencyKey()) == "" || strings.TrimSpace(req.GetHoldId()) == "" {
		return nil, status.Error(codes.InvalidArgument, "idempotency_key and hold_id are required")
	}
	if req.GetAmountMicroUsdc() <= 0 {
		return nil, status.Error(codes.InvalidArgument, "amount must be positive")
	}

	tx, err := s.pg.BeginTx(ctx, pgx.TxOptions{})
	if err != nil {
		return nil, status.Errorf(codes.Internal, "begin tx: %v", err)
	}
	defer tx.Rollback(ctx)

	opExists, err := holdOpExists(ctx, tx, req.GetIdempotencyKey())
	if err != nil {
		return nil, mapPgErr(err)
	}
	if opExists {
		if err := tx.Commit(ctx); err != nil {
			return nil, mapPgErr(err)
		}
		return &emptypb.Empty{}, nil
	}

	var userID, statusStr string
	var amount, committed, released int64
	err = tx.QueryRow(ctx,
		`SELECT user_id, amount_micro_usdc, committed_micro_usdc, released_micro_usdc, status
FROM ledger.holds WHERE hold_id=$1 FOR UPDATE`,
		req.GetHoldId(),
	).Scan(&userID, &amount, &committed, &released, &statusStr)
	if errors.Is(err, pgx.ErrNoRows) {
		return nil, status.Error(codes.NotFound, "hold not found")
	}
	if err != nil {
		return nil, mapPgErr(err)
	}
	if statusStr != "ACTIVE" {
		return nil, status.Error(codes.FailedPrecondition, "hold is not active")
	}

	remaining := amount - committed - released
	if req.GetAmountMicroUsdc() > remaining {
		return nil, status.Error(codes.FailedPrecondition, "release amount exceeds hold remaining")
	}

	cashCode := fmt.Sprintf("LIAB:USER:%s:CASH", userID)
	holdsCode := fmt.Sprintf("LIAB:USER:%s:HOLDS", userID)
	meta := json.RawMessage(`{"op":"RELEASE_HOLD"}`)
	txID, _, _, err := createOrGetTransaction(ctx, tx, req.GetIdempotencyKey(), req.GetReasonCode(), meta)
	if err != nil {
		return nil, mapPgErr(err)
	}

	if err := applyEntries(ctx, tx, []entrySpec{
		{accountCode: holdsCode, direction: "DR", amount: req.GetAmountMicroUsdc(), memo: "release hold"},
		{accountCode: cashCode, direction: "CR", amount: req.GetAmountMicroUsdc(), memo: "release hold"},
	}); err != nil {
		return nil, err
	}

	_, err = tx.Exec(ctx,
		`UPDATE ledger.holds
SET released_micro_usdc = released_micro_usdc + $1,
status = CASE WHEN committed_micro_usdc + released_micro_usdc + $1 >= amount_micro_usdc THEN 'CLOSED' ELSE status END,
closed_at = CASE WHEN committed_micro_usdc + released_micro_usdc + $1 >= amount_micro_usdc THEN now() ELSE closed_at END
WHERE hold_id=$2`,
		req.GetAmountMicroUsdc(), req.GetHoldId(),
	)
	if err != nil {
		return nil, mapPgErr(err)
	}

	_, err = tx.Exec(ctx,
		`INSERT INTO ledger.hold_operations (idempotency_key, hold_id, operation_type, amount_micro_usdc, ledger_tx_id)
VALUES ($1,$2,'RELEASE',$3,$4)`,
		req.GetIdempotencyKey(), req.GetHoldId(), req.GetAmountMicroUsdc(), txID,
	)
	if err != nil {
		return nil, mapPgErr(err)
	}

	if err := insertOutbox(ctx, tx, txID, "LEDGER_TRANSACTION_POSTED", meta); err != nil {
		return nil, mapPgErr(err)
	}
	if err := tx.Commit(ctx); err != nil {
		return nil, mapPgErr(err)
	}
	return &emptypb.Empty{}, nil
}

func (s *ledgerServer) CommitHold(ctx context.Context, req *sarvexv1.CommitHoldRequest) (*emptypb.Empty, error) {
	if strings.TrimSpace(req.GetIdempotencyKey()) == "" || strings.TrimSpace(req.GetHoldId()) == "" {
		return nil, status.Error(codes.InvalidArgument, "idempotency_key and hold_id are required")
	}
	if req.GetCommitAmountMicroUsdc() < 0 || req.GetReleaseAmountMicroUsdc() < 0 {
		return nil, status.Error(codes.InvalidArgument, "commit/release amounts cannot be negative")
	}
	if req.GetCommitAmountMicroUsdc() == 0 && req.GetReleaseAmountMicroUsdc() == 0 {
		return nil, status.Error(codes.InvalidArgument, "at least one of commit/release must be positive")
	}
	if req.GetCommitAmountMicroUsdc() > 0 && strings.TrimSpace(req.GetDestinationAccountCode()) == "" {
		return nil, status.Error(codes.InvalidArgument, "destination_account_code required when committing")
	}

	tx, err := s.pg.BeginTx(ctx, pgx.TxOptions{})
	if err != nil {
		return nil, status.Errorf(codes.Internal, "begin tx: %v", err)
	}
	defer tx.Rollback(ctx)

	opExists, err := holdOpExists(ctx, tx, req.GetIdempotencyKey())
	if err != nil {
		return nil, mapPgErr(err)
	}
	if opExists {
		if err := tx.Commit(ctx); err != nil {
			return nil, mapPgErr(err)
		}
		return &emptypb.Empty{}, nil
	}

	var userID, statusStr string
	var amount, committed, released int64
	err = tx.QueryRow(ctx,
		`SELECT user_id, amount_micro_usdc, committed_micro_usdc, released_micro_usdc, status
FROM ledger.holds WHERE hold_id=$1 FOR UPDATE`,
		req.GetHoldId(),
	).Scan(&userID, &amount, &committed, &released, &statusStr)
	if errors.Is(err, pgx.ErrNoRows) {
		return nil, status.Error(codes.NotFound, "hold not found")
	}
	if err != nil {
		return nil, mapPgErr(err)
	}
	if statusStr != "ACTIVE" {
		return nil, status.Error(codes.FailedPrecondition, "hold is not active")
	}

	totalDelta := req.GetCommitAmountMicroUsdc() + req.GetReleaseAmountMicroUsdc()
	remaining := amount - committed - released
	if totalDelta > remaining {
		return nil, status.Error(codes.FailedPrecondition, "commit+release exceeds hold remaining")
	}

	holdsCode := fmt.Sprintf("LIAB:USER:%s:HOLDS", userID)
	cashCode := fmt.Sprintf("LIAB:USER:%s:CASH", userID)
	entries := make([]entrySpec, 0, 4+len(req.GetAdditionalEntries()))
	if req.GetCommitAmountMicroUsdc() > 0 {
		if _, err := ensureAccount(ctx, tx, req.GetDestinationAccountCode(), "LIABILITY", "USDC", ""); err != nil {
			return nil, mapPgErr(err)
		}
		entries = append(entries,
			entrySpec{accountCode: holdsCode, direction: "DR", amount: req.GetCommitAmountMicroUsdc(), memo: "commit hold"},
			entrySpec{accountCode: req.GetDestinationAccountCode(), direction: "CR", amount: req.GetCommitAmountMicroUsdc(), memo: "commit hold"},
		)
	}
	if req.GetReleaseAmountMicroUsdc() > 0 {
		entries = append(entries,
			entrySpec{accountCode: holdsCode, direction: "DR", amount: req.GetReleaseAmountMicroUsdc(), memo: "release hold (commit)"},
			entrySpec{accountCode: cashCode, direction: "CR", amount: req.GetReleaseAmountMicroUsdc(), memo: "release hold (commit)"},
		)
	}
	for _, ae := range req.GetAdditionalEntries() {
		dir := strings.ToUpper(strings.TrimSpace(ae.GetDirection()))
		if dir != "DR" && dir != "CR" {
			return nil, status.Error(codes.InvalidArgument, "additional_entries direction must be DR or CR")
		}
		if ae.GetAmountMicroUsdc() <= 0 {
			return nil, status.Error(codes.InvalidArgument, "additional_entries amount must be positive")
		}
		entries = append(entries, entrySpec{accountCode: ae.GetAccountCode(), direction: dir, amount: ae.GetAmountMicroUsdc(), memo: ae.GetMemo()})
	}

	meta := json.RawMessage(`{"op":"COMMIT_HOLD"}`)
	txID, _, _, err := createOrGetTransaction(ctx, tx, req.GetIdempotencyKey(), req.GetReasonCode(), meta)
	if err != nil {
		return nil, mapPgErr(err)
	}
	if err := applyEntries(ctx, tx, entries); err != nil {
		return nil, err
	}

	_, err = tx.Exec(ctx,
		`UPDATE ledger.holds
SET committed_micro_usdc = committed_micro_usdc + $1,
released_micro_usdc = released_micro_usdc + $2,
status = CASE WHEN committed_micro_usdc + released_micro_usdc + $1 + $2 >= amount_micro_usdc THEN 'CLOSED' ELSE status END,
closed_at = CASE WHEN committed_micro_usdc + released_micro_usdc + $1 + $2 >= amount_micro_usdc THEN now() ELSE closed_at END
WHERE hold_id=$3`,
		req.GetCommitAmountMicroUsdc(), req.GetReleaseAmountMicroUsdc(), req.GetHoldId(),
	)
	if err != nil {
		return nil, mapPgErr(err)
	}

	_, err = tx.Exec(ctx,
		`INSERT INTO ledger.hold_operations (idempotency_key, hold_id, operation_type, amount_micro_usdc, ledger_tx_id)
VALUES ($1,$2,'COMMIT',$3,$4)`,
		req.GetIdempotencyKey(), req.GetHoldId(), totalDelta, txID,
	)
	if err != nil {
		return nil, mapPgErr(err)
	}

	if err := insertOutbox(ctx, tx, txID, "LEDGER_TRANSACTION_POSTED", meta); err != nil {
		return nil, mapPgErr(err)
	}
	if err := tx.Commit(ctx); err != nil {
		return nil, mapPgErr(err)
	}
	return &emptypb.Empty{}, nil
}

func (s *ledgerServer) GetBalance(ctx context.Context, req *sarvexv1.GetBalanceRequest) (*sarvexv1.Balance, error) {
	if strings.TrimSpace(req.GetUserId()) == "" {
		return nil, status.Error(codes.InvalidArgument, "user_id is required")
	}

	var cash, held int64
	userCash := fmt.Sprintf("LIAB:USER:%s:CASH", req.GetUserId())
	userHolds := fmt.Sprintf("LIAB:USER:%s:HOLDS", req.GetUserId())
	err := s.pg.QueryRow(ctx, `SELECT
COALESCE((
  SELECT running_balance_micro_usdc
  FROM ledger.entries
  WHERE account_id = (SELECT account_id FROM ledger.accounts WHERE account_code=$1)
  ORDER BY account_seq DESC
  LIMIT 1
), 0),
COALESCE((
  SELECT running_balance_micro_usdc
  FROM ledger.entries
  WHERE account_id = (SELECT account_id FROM ledger.accounts WHERE account_code=$2)
  ORDER BY account_seq DESC
  LIMIT 1
), 0)`, userCash, userHolds).Scan(&cash, &held)
	if err != nil {
		return nil, mapPgErr(err)
	}

	return &sarvexv1.Balance{UserId: req.GetUserId(), CashMicroUsdc: cash, HeldMicroUsdc: held, TotalMicroUsdc: cash + held}, nil
}

func (s *ledgerServer) GetAccountHistory(ctx context.Context, req *sarvexv1.GetAccountHistoryRequest) (*sarvexv1.GetAccountHistoryResponse, error) {
	if strings.TrimSpace(req.GetUserId()) == "" {
		return nil, status.Error(codes.InvalidArgument, "user_id is required")
	}
	limit := int(req.GetLimit())
	if limit <= 0 || limit > 500 {
		limit = 100
	}

	rows, err := s.pg.Query(ctx,
		`SELECT t.tx_id, a.account_code, e.direction, e.amount_micro_usdc, e.running_balance_micro_usdc,
t.reason_code, e.posted_at, COALESCE(e.memo, '')
FROM ledger.entries e
JOIN ledger.accounts a ON a.account_id = e.account_id
JOIN ledger.transactions t ON t.tx_id = e.tx_id
WHERE a.user_id = $1
ORDER BY e.entry_id DESC
LIMIT $2`,
		req.GetUserId(), limit,
	)
	if err != nil {
		return nil, mapPgErr(err)
	}
	defer rows.Close()

	resp := &sarvexv1.GetAccountHistoryResponse{}
	for rows.Next() {
		var txID int64
		var accountCode, direction, reasonCode, memo string
		var amount, running int64
		var postedAt time.Time
		if err := rows.Scan(&txID, &accountCode, &direction, &amount, &running, &reasonCode, &postedAt, &memo); err != nil {
			return nil, mapPgErr(err)
		}
		resp.Entries = append(resp.Entries, &sarvexv1.LedgerEntryRecord{
			TxId:                    fmt.Sprintf("%d", txID),
			AccountCode:             accountCode,
			Direction:               direction,
			AmountMicroUsdc:         amount,
			RunningBalanceMicroUsdc: running,
			ReasonCode:              reasonCode,
			PostedAt:                timestamppb.New(postedAt),
			Memo:                    memo,
		})
	}
	return resp, nil
}

func (s *ledgerServer) AdminCreditDeposit(ctx context.Context, req *sarvexv1.AdminCreditDepositRequest) (*emptypb.Empty, error) {
	if strings.TrimSpace(req.GetUserId()) == "" || req.GetAmountMicroUsdc() <= 0 {
		return nil, status.Error(codes.InvalidArgument, "user_id and positive amount required")
	}

	tx, err := s.pg.BeginTx(ctx, pgx.TxOptions{})
	if err != nil {
		return nil, status.Errorf(codes.Internal, "begin tx: %v", err)
	}
	defer tx.Rollback(ctx)

	houseWallet := "ASSET:HOUSE:WALLET"
	userCash := fmt.Sprintf("LIAB:USER:%s:CASH", req.GetUserId())
	if _, err := ensureAccount(ctx, tx, houseWallet, "ASSET", "USDC", ""); err != nil {
		return nil, mapPgErr(err)
	}
	if _, err := ensureAccount(ctx, tx, userCash, "LIABILITY", "USDC", req.GetUserId()); err != nil {
		return nil, mapPgErr(err)
	}
	if _, err := ensureAccount(ctx, tx, fmt.Sprintf("LIAB:USER:%s:HOLDS", req.GetUserId()), "LIABILITY", "USDC", req.GetUserId()); err != nil {
		return nil, mapPgErr(err)
	}

	idem := fmt.Sprintf("admin_deposit:%s:%d", req.GetUserId(), time.Now().UnixNano())
	meta := json.RawMessage(`{"op":"ADMIN_CREDIT_DEPOSIT"}`)
	txID, _, _, err := createOrGetTransaction(ctx, tx, idem, "DEPOSIT", meta)
	if err != nil {
		return nil, mapPgErr(err)
	}
	if err := applyEntries(ctx, tx, []entrySpec{
		{accountCode: houseWallet, direction: "DR", amount: req.GetAmountMicroUsdc(), memo: req.GetNote()},
		{accountCode: userCash, direction: "CR", amount: req.GetAmountMicroUsdc(), memo: req.GetNote()},
	}); err != nil {
		return nil, err
	}
	if err := insertOutbox(ctx, tx, txID, "LEDGER_TRANSACTION_POSTED", meta); err != nil {
		return nil, mapPgErr(err)
	}
	if err := tx.Commit(ctx); err != nil {
		return nil, mapPgErr(err)
	}
	return &emptypb.Empty{}, nil
}

func applyEntries(ctx context.Context, tx pgx.Tx, entries []entrySpec) error {
	states := map[string]*accountState{}
	for _, e := range entries {
		if strings.TrimSpace(e.accountCode) == "" {
			return status.Error(codes.InvalidArgument, "account_code is required")
		}
		if _, err := ensureAccount(ctx, tx, e.accountCode, inferAccountType(e.accountCode), "USDC", inferUserID(e.accountCode)); err != nil {
			return mapPgErr(err)
		}
	}

	accountCodes := uniqueSortedCodes(entries)
	for _, code := range accountCodes {
		acctID, bal, nextSeq, err := lockAccountState(ctx, tx, code)
		if err != nil {
			return mapPgErr(err)
		}
		states[code] = &accountState{accountID: acctID, balance: bal, nextSeq: nextSeq}
	}

	for _, e := range entries {
		st := states[e.accountCode]
		newBal := st.balance
		if e.direction == "DR" {
			newBal -= e.amount
		} else {
			newBal += e.amount
		}
		if strings.Contains(e.accountCode, ":CASH") || strings.Contains(e.accountCode, ":HOLDS") {
			if newBal < 0 {
				return status.Error(codes.FailedPrecondition, "insufficient funds")
			}
		}
		st.nextSeq++
		_, err := tx.Exec(ctx,
			`INSERT INTO ledger.entries (tx_id, account_id, direction, amount_micro_usdc, running_balance_micro_usdc, account_seq, memo)
VALUES (currval('ledger.transactions_tx_id_seq'), $1, $2, $3, $4, $5, $6)`,
			st.accountID, e.direction, e.amount, newBal, st.nextSeq, e.memo,
		)
		if err != nil {
			return mapPgErr(err)
		}
		st.balance = newBal
	}
	return nil
}

func createOrGetTransaction(ctx context.Context, tx pgx.Tx, idemKey, reasonCode string, metadata json.RawMessage) (int64, time.Time, bool, error) {
	if strings.TrimSpace(reasonCode) == "" {
		reasonCode = "UNSPECIFIED"
	}
	var txID int64
	var postedAt time.Time
	err := tx.QueryRow(ctx,
		`INSERT INTO ledger.transactions (idempotency_key, reason_code, metadata)
VALUES ($1,$2,$3)
ON CONFLICT (idempotency_key) DO NOTHING
RETURNING tx_id, posted_at`,
		idemKey, reasonCode, metadata,
	).Scan(&txID, &postedAt)
	if err == nil {
		return txID, postedAt, false, nil
	}
	if !errors.Is(err, pgx.ErrNoRows) {
		return 0, time.Time{}, false, err
	}

	err = tx.QueryRow(ctx,
		`SELECT tx_id, posted_at FROM ledger.transactions WHERE idempotency_key=$1`,
		idemKey,
	).Scan(&txID, &postedAt)
	if err != nil {
		return 0, time.Time{}, false, err
	}
	return txID, postedAt, true, nil
}

func ensureAccount(ctx context.Context, tx pgx.Tx, code, acctType, currency, userID string) (int64, error) {
	var id int64
	err := tx.QueryRow(ctx,
		`INSERT INTO ledger.accounts (account_code, account_type, currency, user_id)
VALUES ($1,$2::ledger.account_type,$3,NULLIF($4,''))
ON CONFLICT (account_code) DO NOTHING
RETURNING account_id`,
		code, acctType, currency, userID,
	).Scan(&id)
	if err == nil {
		return id, nil
	}
	if !errors.Is(err, pgx.ErrNoRows) {
		return 0, err
	}
	err = tx.QueryRow(ctx, `SELECT account_id FROM ledger.accounts WHERE account_code=$1`, code).Scan(&id)
	return id, err
}

func lockAccountState(ctx context.Context, tx pgx.Tx, code string) (int64, int64, int64, error) {
	var accountID int64
	if err := tx.QueryRow(ctx, `SELECT account_id FROM ledger.accounts WHERE account_code=$1 FOR UPDATE`, code).Scan(&accountID); err != nil {
		return 0, 0, 0, err
	}
	var bal, seq int64
	err := tx.QueryRow(ctx,
		`SELECT COALESCE(running_balance_micro_usdc,0), COALESCE(account_seq,0)
FROM ledger.entries WHERE account_id=$1 ORDER BY account_seq DESC LIMIT 1`,
		accountID,
	).Scan(&bal, &seq)
	if errors.Is(err, pgx.ErrNoRows) {
		return accountID, 0, 0, nil
	}
	if err != nil {
		return 0, 0, 0, err
	}
	return accountID, bal, seq, nil
}

func holdOpExists(ctx context.Context, tx pgx.Tx, idemKey string) (bool, error) {
	var exists bool
	err := tx.QueryRow(ctx, `SELECT EXISTS(SELECT 1 FROM ledger.hold_operations WHERE idempotency_key=$1)`, idemKey).Scan(&exists)
	return exists, err
}

func insertOutbox(ctx context.Context, tx pgx.Tx, txID int64, eventType string, payload json.RawMessage) error {
	_, err := tx.Exec(ctx,
		`INSERT INTO ledger.ledger_event_outbox (tx_id, event_type, payload)
VALUES ($1,$2,$3)
ON CONFLICT (tx_id, event_type) DO NOTHING`,
		txID, eventType, payload,
	)
	return err
}

func inferAccountType(code string) string {
	switch {
	case strings.HasPrefix(code, "ASSET:"):
		return "ASSET"
	case strings.HasPrefix(code, "LIAB:"):
		return "LIABILITY"
	case strings.HasPrefix(code, "REVENUE:"):
		return "REVENUE"
	case strings.HasPrefix(code, "EXPENSE:"):
		return "EXPENSE"
	default:
		return "EQUITY"
	}
}

func inferUserID(code string) string {
	parts := strings.Split(code, ":")
	if len(parts) >= 4 && parts[0] == "LIAB" && parts[1] == "USER" {
		return parts[2]
	}
	return ""
}

func uniqueSortedCodes(entries []entrySpec) []string {
	m := map[string]struct{}{}
	for _, e := range entries {
		m[e.accountCode] = struct{}{}
	}
	codes := make([]string, 0, len(m))
	for k := range m {
		codes = append(codes, k)
	}
	sort.Strings(codes)
	return codes
}

func structToJSON(v any) (json.RawMessage, error) {
	if v == nil {
		return json.RawMessage(`{}`), nil
	}
	b, err := json.Marshal(v)
	if err != nil {
		return nil, err
	}
	if len(b) == 0 {
		return json.RawMessage(`{}`), nil
	}
	return b, nil
}

func mapPgErr(err error) error {
	if err == nil {
		return nil
	}
	var pgErr *pgconn.PgError
	if errors.As(err, &pgErr) {
		if pgErr.Code == "23505" {
			return status.Error(codes.AlreadyExists, pgErr.Message)
		}
		if pgErr.Code == "23514" {
			return status.Error(codes.FailedPrecondition, pgErr.Message)
		}
	}
	if errors.Is(err, pgx.ErrNoRows) {
		return status.Error(codes.NotFound, "not found")
	}
	return status.Errorf(codes.Internal, "%v", err)
}
