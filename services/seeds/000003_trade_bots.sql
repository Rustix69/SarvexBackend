INSERT INTO risk.user_limits (
  user_id,
  kyc_tier,
  max_order_size_micro_usdc,
  daily_loss_limit_micro_usdc
)
SELECT
  'u_bot_' || lpad(bot_number::text, 3, '0'),
  2,
  10000000000,
  100000000000
FROM generate_series(1, 50) AS bot_number
ON CONFLICT (user_id) DO UPDATE SET
  kyc_tier = EXCLUDED.kyc_tier,
  max_order_size_micro_usdc = EXCLUDED.max_order_size_micro_usdc,
  daily_loss_limit_micro_usdc = EXCLUDED.daily_loss_limit_micro_usdc,
  updated_at = now();
