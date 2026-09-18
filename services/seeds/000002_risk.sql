INSERT INTO risk.user_limits (user_id, kyc_tier, max_order_size_micro_usdc, daily_loss_limit_micro_usdc)
VALUES
  ('u_retail_1', 1, 10000000000, 100000000000),
  ('u_inst_1', 2, 10000000000, 100000000000),
  ('u_mm_1', 2, 100000000000, 1000000000000),
  ('u_admin', 99, 100000000000, 1000000000000)
ON CONFLICT (user_id) DO UPDATE SET
  kyc_tier = EXCLUDED.kyc_tier,
  max_order_size_micro_usdc = EXCLUDED.max_order_size_micro_usdc,
  daily_loss_limit_micro_usdc = EXCLUDED.daily_loss_limit_micro_usdc,
  updated_at = now();
