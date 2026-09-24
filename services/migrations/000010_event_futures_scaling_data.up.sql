-- The legacy catalog used 1,000 micro-USDC for nearly every scalar contract,
-- regardless of the contract's display unit. Replace that placeholder with the
-- documented display multiplier and persist the derived raw-tick value.
WITH scaling(ticker, divider, multiplier_micro) AS (
  VALUES
    ('SXF-HIGHNY-26OCT15', 10, 10000000),
    ('SXF-RAINPHL-26OCT15', 100, 100000000),
    ('SXF-WTI-26OCT30', 100, 10000000),
    ('SXF-WTIHI-26OCT', 100, 10000000),
    ('SXF-XAU-26OCT16', 10, 1000000),
    ('SXF-BTCD-26OCT30H17', 1, 10000),
    ('SXF-BTCH-26OCT30-1700', 1, 10000),
    ('SXF-BTCLOW-26DEC31', 1, 10000),
    ('SXF-ETHD-26OCT30H17', 1, 100000),
    ('SXF-FFUB-26OCT', 100, 100000000),
    ('SXF-FFUB-26DEC', 100, 100000000),
    ('SXF-USCPIYOY-26SEP', 100, 100000000),
    ('SXF-USCPIMOM-26SEP', 100, 100000000),
    ('SXF-AAAGAS-26OCT19', 1000, 100000000),
    ('SXF-NFP-26SEP', 100, 100000),
    ('SXF-USHOUSE-R-26', 1, 1000000),
    ('SXF-USSENATE-R-26', 1, 5000000),
    ('SXF-SPX-26OCT16', 1, 1000000),
    ('SXF-NDX-26OCT16', 1, 200000),
    ('SXF-SPXY-26DEC31', 1, 1000000),
    ('SXF-UST10Y-26OCT30', 100, 100000000),
    ('SXF-EURUSD-26OCT16', 10000, 1000000000),
    ('SXF-HORMUZW-26OCT18', 1, 1000000),
    ('SXF-HORMUZMA-26DEC31', 1, 5000000),
    ('SXF-BOJRATE-26SEP', 100, 100000000),
    ('SXF-USDJPY-26OCT30', 100, 100000000),
    ('SXF-N225-26OCT30', 1, 10000),
    ('SXF-LPR1Y-26OCT', 100, 100000000),
    ('SXF-HIGHTYO-26OCT01', 10, 10000000),
    ('SXF-BTCASIA-26OCT30H0800Z', 1, 10000),
    ('SXF-ECBDFR-26OCT', 100, 100000000),
    ('SXF-EZHICP-26OCT', 100, 100000000),
    ('SXF-TTF-26OCT30', 100, 10000000),
    ('SXF-FRPRES27-RN', 100, 10000000),
    ('SXF-RBIREPO-26OCT', 100, 100000000),
    ('SXF-INCPI-26SEP', 100, 100000000),
    ('SXF-USDINR-26OCT30', 100, 100000000),
    ('SXF-NIFTY-26OCT27', 1, 100000),
    ('SXF-MONSOON-26', 100, 10000000),
    ('SXF-DELAQI-26NOV09', 1, 100000),
    ('SXF-LBMAGOLD-26NOV06', 10, 1000000),
    ('SXF-UP27-BJP', 100, 1000000),
    ('SXF-BRENT-26OCT30', 100, 10000000),
    ('SXF-CBUAE-26OCT', 100, 100000000),
    ('SXF-OPECP-26DEC', 1, 100000),
    ('SXF-HIGHDXB-26OCT15', 10, 10000000),
    ('SXF-TASI-26OCT29', 1, 100000)
)
UPDATE refdata.contracts AS c
SET divider = scaling.divider,
    multiplier_micro_per_display_unit = scaling.multiplier_micro,
    tick_value_micro = scaling.multiplier_micro / scaling.divider,
    updated_at = now()
FROM scaling
WHERE c.ticker = scaling.ticker
  AND c.kind = 'SCALAR'::refdata.contract_kind;

UPDATE refdata.contracts
SET divider = COALESCE(NULLIF((settlement_rule #>> '{encoding,divider}')::BIGINT, 0), 1),
    tick_value_micro = CASE
      WHEN kind = 'BINARY'::refdata.contract_kind THEN COALESCE(multiplier_micro_usdc, 0)
      ELSE tick_value_micro
    END,
    multiplier_micro_per_display_unit = CASE
      WHEN kind = 'BINARY'::refdata.contract_kind THEN multiplier_micro_usdc
      ELSE multiplier_micro_per_display_unit
    END
WHERE kind = 'BINARY'::refdata.contract_kind;

DO $$
BEGIN
  IF EXISTS (
    SELECT 1 FROM refdata.contracts
    WHERE kind = 'SCALAR'::refdata.contract_kind
      AND (divider <= 0 OR multiplier_micro_per_display_unit IS NULL OR multiplier_micro_per_display_unit <= 0 OR tick_value_micro <= 0)
  ) THEN
    RAISE EXCEPTION 'scalar contract scaling is incomplete';
  END IF;
END $$;

ALTER TABLE refdata.contracts
  DROP CONSTRAINT IF EXISTS contracts_scalar_scaling_check;

ALTER TABLE refdata.contracts
  ADD CONSTRAINT contracts_scalar_scaling_check CHECK (
    kind <> 'SCALAR'::refdata.contract_kind
    OR (divider > 0 AND multiplier_micro_per_display_unit > 0 AND tick_value_micro > 0)
  );
