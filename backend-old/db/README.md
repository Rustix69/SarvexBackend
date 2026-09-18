# Database Directory

## Milestone 02 Status: Completed

- Canonical migration added:
  - `db/migrations/000001_milestone2_init.up.sql`
  - `db/migrations/000001_milestone2_init.down.sql`
- Includes service-owned schemas:
  - `refdata`, `users`, `ledger`, `orders`, `risk`, `position`, `oracle`, `settlement`, `audit`
- Includes core correctness structures:
  - ledger balanced-transaction constraint trigger
  - append-only protection for `ledger.entries` (blocks UPDATE/DELETE)
  - hold lifecycle tables (`ledger.holds`, `ledger.hold_operations`)
  - order fill + outbox tables (`orders.fills`, `orders.fill_posting_outbox`)
  - position replay tables (`position.consumer_offsets`, `position.applied_fills`)
  - settlement payout idempotency (`settlement.settlement_payouts.idempotency_key` unique)
- Seeds implemented:
  - `db/seed/contracts.sql`
  - `db/seed/demo_users.sql`
  - `db/seed/house_accounts.sql`
- Seed runner wired:
  - `scripts/seed-demo-data.sh`
