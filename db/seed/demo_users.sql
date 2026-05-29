INSERT INTO users.users (user_id, email, display_name, password_hash, kyc_tier, is_admin, is_mm) VALUES
  ('u_retail_1', 'retail@demo.sarvex.com', 'Demo Retail',
   '$2a$10$N9qo8uLOickgx2ZMRZoMyeIjZAgcfl7p92ldGxad68LJZdL17lhWy', 1, false, false),
  ('u_inst_1', 'inst@demo.sarvex.com', 'Demo Institutional',
   '$2a$10$N9qo8uLOickgx2ZMRZoMyeIjZAgcfl7p92ldGxad68LJZdL17lhWy', 2, false, false),
  ('u_mm_1', 'mm@demo.sarvex.com', 'Demo MM Bot',
   '$2a$10$N9qo8uLOickgx2ZMRZoMyeIjZAgcfl7p92ldGxad68LJZdL17lhWy', 2, false, true),
  ('u_admin', 'admin@demo.sarvex.com', 'Demo Admin',
   '$2a$10$N9qo8uLOickgx2ZMRZoMyeIjZAgcfl7p92ldGxad68LJZdL17lhWy', 99, true, false)
ON CONFLICT (user_id) DO NOTHING;

INSERT INTO risk.user_limits (user_id, kyc_tier, max_order_size_micro_usdc, daily_loss_limit_micro_usdc, orders_per_second_limit) VALUES
  ('u_retail_1', 1, 10000000000, 100000000000, 25),
  ('u_inst_1', 2, 10000000000, 100000000000, 50),
  ('u_mm_1', 2, 100000000000, 1000000000000, 200),
  ('u_admin', 99, 100000000000, 1000000000000, 500)
ON CONFLICT (user_id) DO UPDATE SET
  kyc_tier=EXCLUDED.kyc_tier,
  max_order_size_micro_usdc=EXCLUDED.max_order_size_micro_usdc,
  daily_loss_limit_micro_usdc=EXCLUDED.daily_loss_limit_micro_usdc,
  orders_per_second_limit=EXCLUDED.orders_per_second_limit,
  updated_at=now();
