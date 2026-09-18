CREATE SCHEMA IF NOT EXISTS ledger;

DO $$ BEGIN
  CREATE TYPE ledger.account_type AS ENUM ('ASSET', 'LIABILITY', 'EQUITY', 'REVENUE', 'EXPENSE');
EXCEPTION WHEN duplicate_object THEN NULL;
END $$;

CREATE TABLE IF NOT EXISTS ledger.accounts (
  account_id BIGSERIAL PRIMARY KEY,
  account_code TEXT NOT NULL UNIQUE,
  account_type ledger.account_type NOT NULL,
  currency TEXT NOT NULL DEFAULT 'USDC',
  user_id TEXT,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_ledger_accounts_user_id ON ledger.accounts(user_id) WHERE user_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS ledger.transactions (
  tx_id BIGSERIAL PRIMARY KEY,
  idempotency_key TEXT NOT NULL UNIQUE,
  reason_code TEXT NOT NULL,
  metadata JSONB NOT NULL DEFAULT '{}',
  posted_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_ledger_transactions_posted_at ON ledger.transactions(posted_at);

CREATE TABLE IF NOT EXISTS ledger.entries (
  entry_id BIGSERIAL PRIMARY KEY,
  tx_id BIGINT NOT NULL REFERENCES ledger.transactions(tx_id),
  account_id BIGINT NOT NULL REFERENCES ledger.accounts(account_id),
  direction CHAR(2) NOT NULL CHECK (direction IN ('DR', 'CR')),
  amount_micro_usdc BIGINT NOT NULL CHECK (amount_micro_usdc > 0),
  running_balance_micro_usdc BIGINT NOT NULL,
  account_seq BIGINT NOT NULL,
  memo TEXT,
  posted_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  UNIQUE(account_id, account_seq)
);
CREATE INDEX IF NOT EXISTS idx_ledger_entries_tx_id ON ledger.entries(tx_id);
CREATE INDEX IF NOT EXISTS idx_ledger_entries_account ON ledger.entries(account_id, entry_id DESC);

CREATE OR REPLACE FUNCTION ledger.assert_tx_balanced()
RETURNS trigger AS $$
DECLARE imbalance BIGINT;
BEGIN
  SELECT COALESCE(SUM(CASE WHEN direction = 'DR' THEN amount_micro_usdc ELSE 0 END), 0)
       - COALESCE(SUM(CASE WHEN direction = 'CR' THEN amount_micro_usdc ELSE 0 END), 0)
    INTO imbalance
    FROM ledger.entries WHERE tx_id = NEW.tx_id;
  IF imbalance <> 0 THEN
    RAISE EXCEPTION 'ledger transaction % is unbalanced by %', NEW.tx_id, imbalance;
  END IF;
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_assert_tx_balanced ON ledger.entries;
CREATE CONSTRAINT TRIGGER trg_assert_tx_balanced
  AFTER INSERT ON ledger.entries DEFERRABLE INITIALLY DEFERRED
  FOR EACH ROW EXECUTE FUNCTION ledger.assert_tx_balanced();

CREATE OR REPLACE FUNCTION ledger.reject_entry_mutation()
RETURNS trigger AS $$
BEGIN
  RAISE EXCEPTION 'ledger.entries is append-only';
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_reject_entry_update ON ledger.entries;
DROP TRIGGER IF EXISTS trg_reject_entry_delete ON ledger.entries;
CREATE TRIGGER trg_reject_entry_update BEFORE UPDATE ON ledger.entries FOR EACH ROW EXECUTE FUNCTION ledger.reject_entry_mutation();
CREATE TRIGGER trg_reject_entry_delete BEFORE DELETE ON ledger.entries FOR EACH ROW EXECUTE FUNCTION ledger.reject_entry_mutation();

CREATE TABLE IF NOT EXISTS ledger.ledger_event_outbox (
  outbox_id BIGSERIAL PRIMARY KEY,
  tx_id BIGINT NOT NULL REFERENCES ledger.transactions(tx_id),
  event_type TEXT NOT NULL,
  payload JSONB NOT NULL,
  status TEXT NOT NULL DEFAULT 'PENDING' CHECK (status IN ('PENDING', 'POSTED')),
  attempts INTEGER NOT NULL DEFAULT 0,
  next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  UNIQUE(tx_id, event_type)
);
CREATE INDEX IF NOT EXISTS idx_ledger_outbox_pending ON ledger.ledger_event_outbox(status, next_attempt_at, outbox_id);

CREATE TABLE IF NOT EXISTS ledger.holds (
  hold_id TEXT PRIMARY KEY,
  user_id TEXT NOT NULL,
  amount_micro_usdc BIGINT NOT NULL CHECK (amount_micro_usdc > 0),
  committed_micro_usdc BIGINT NOT NULL DEFAULT 0 CHECK (committed_micro_usdc >= 0),
  released_micro_usdc BIGINT NOT NULL DEFAULT 0 CHECK (released_micro_usdc >= 0),
  reason TEXT NOT NULL,
  status TEXT NOT NULL DEFAULT 'ACTIVE' CHECK (status IN ('ACTIVE', 'CLOSED')),
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  closed_at TIMESTAMPTZ,
  CHECK (committed_micro_usdc + released_micro_usdc <= amount_micro_usdc)
);
CREATE INDEX IF NOT EXISTS idx_ledger_holds_user_status ON ledger.holds(user_id, status);

CREATE TABLE IF NOT EXISTS ledger.hold_operations (
  idempotency_key TEXT PRIMARY KEY,
  hold_id TEXT NOT NULL REFERENCES ledger.holds(hold_id),
  operation_type TEXT NOT NULL CHECK (operation_type IN ('PLACE', 'RELEASE', 'COMMIT')),
  amount_micro_usdc BIGINT NOT NULL CHECK (amount_micro_usdc >= 0),
  ledger_tx_id BIGINT REFERENCES ledger.transactions(tx_id),
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_ledger_hold_operations_hold_id ON ledger.hold_operations(hold_id);

CREATE OR REPLACE VIEW ledger.user_balances AS
SELECT a.user_id,
  COALESCE(SUM(CASE WHEN a.account_code LIKE 'LIAB:USER:%:CASH' THEN latest.running_balance_micro_usdc ELSE 0 END), 0)::BIGINT AS cash_micro_usdc,
  COALESCE(SUM(CASE WHEN a.account_code LIKE 'LIAB:USER:%:HOLDS' THEN latest.running_balance_micro_usdc ELSE 0 END), 0)::BIGINT AS held_micro_usdc
FROM ledger.accounts a
LEFT JOIN LATERAL (
  SELECT running_balance_micro_usdc FROM ledger.entries e
  WHERE e.account_id = a.account_id ORDER BY e.entry_id DESC LIMIT 1
) latest ON TRUE
WHERE a.user_id IS NOT NULL
GROUP BY a.user_id;
