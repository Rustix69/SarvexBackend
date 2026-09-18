-- Demo futures contract conventions. Values are integer price ticks.
-- This seed is intentionally separate from the workbook import so a future
-- workbook refresh cannot silently restore placeholder 1..1,000,000 bounds.

UPDATE refdata.contracts
SET tick_size=1, min_price_ticks=1, max_price_ticks=1000,
    lower_bound_ticks=1, upper_bound_ticks=1000,
    multiplier_micro_usdc=100000, updated_at=now()
WHERE kind='SCALAR'
  AND min_price_ticks=1 AND max_price_ticks=1000000
  AND ticker LIKE 'SXF-%';

UPDATE refdata.contracts
SET tick_size=1, min_price_ticks=600, max_price_ticks=1200,
    lower_bound_ticks=600, upper_bound_ticks=1200,
    multiplier_micro_usdc=500000, updated_at=now()
WHERE ticker IN ('SXF-FFUB-26OCT','SXF-FFUB-26DEC');

UPDATE refdata.contracts
SET tick_size=1, min_price_ticks=300, max_price_ticks=500,
    lower_bound_ticks=300, upper_bound_ticks=500,
    multiplier_micro_usdc=1000000, updated_at=now()
WHERE ticker IN ('SXF-USCPIYOY-26SEP','SXF-INCPI-26SEP','SXF-EZHICP-26OCT');

UPDATE refdata.contracts
SET tick_size=1, min_price_ticks=0, max_price_ticks=100,
    lower_bound_ticks=0, upper_bound_ticks=100,
    multiplier_micro_usdc=1000000, updated_at=now()
WHERE ticker='SXF-USCPIMOM-26SEP';

UPDATE refdata.contracts
SET tick_size=1, min_price_ticks=200, max_price_ticks=650,
    lower_bound_ticks=200, upper_bound_ticks=650,
    multiplier_micro_usdc=1000000, updated_at=now()
WHERE ticker IN ('SXF-UST10Y-26OCT30','SXF-RBIREPO-26OCT','SXF-CBUAE-26OCT','SXF-BOJRATE-26SEP','SXF-LPR1Y-26OCT','SXF-ECBDFR-26OCT');

UPDATE refdata.contracts
SET tick_size=1, min_price_ticks=0, max_price_ticks=1000,
    lower_bound_ticks=0, upper_bound_ticks=1000,
    multiplier_micro_usdc=1000000, updated_at=now()
WHERE ticker IN ('SXF-FRPRES27-RN','SXF-MONSOON-26','SXF-AG26-IND');

UPDATE refdata.contracts
SET tick_size=1, min_price_ticks=250, max_price_ticks=600,
    lower_bound_ticks=250, upper_bound_ticks=600,
    multiplier_micro_usdc=1000000, updated_at=now()
WHERE ticker='SXF-FRPRES27-RN';

UPDATE refdata.contracts
SET tick_size=25, min_price_ticks=6000, max_price_ticks=9000,
    lower_bound_ticks=6000, upper_bound_ticks=9000,
    multiplier_micro_usdc=1000000, updated_at=now()
WHERE ticker IN ('SXF-SPX-26OCT16','SXF-SPXY-26DEC31');

UPDATE refdata.contracts
SET tick_size=25, min_price_ticks=20000, max_price_ticks=35000,
    lower_bound_ticks=20000, upper_bound_ticks=35000,
    multiplier_micro_usdc=200000, updated_at=now()
WHERE ticker='SXF-NDX-26OCT16';

UPDATE refdata.contracts
SET tick_size=5, min_price_ticks=18000, max_price_ticks=32000,
    lower_bound_ticks=18000, upper_bound_ticks=32000,
    multiplier_micro_usdc=100000, updated_at=now()
WHERE ticker IN ('SXF-NIFTY-26OCT27','SXF-N225-26OCT30');

UPDATE refdata.contracts
SET tick_size=1, min_price_ticks=60000, max_price_ticks=120000,
    lower_bound_ticks=60000, upper_bound_ticks=120000,
    multiplier_micro_usdc=10000, updated_at=now()
WHERE ticker IN ('SXF-BTCD-26OCT30H17','SXF-BTCH-{hour}','SXF-BTCLOW-26DEC31','SXF-BTCASIA-26OCT30H0800Z');

UPDATE refdata.contracts
SET tick_size=1, min_price_ticks=2000, max_price_ticks=6000,
    lower_bound_ticks=2000, upper_bound_ticks=6000,
    multiplier_micro_usdc=100000, updated_at=now()
WHERE ticker='SXF-ETHD-26OCT30H17';

UPDATE refdata.contracts
SET tick_size=1, min_price_ticks=2500, max_price_ticks=6000,
    lower_bound_ticks=2500, upper_bound_ticks=6000,
    multiplier_micro_usdc=100000, updated_at=now()
WHERE ticker='SXF-AAAGAS-26OCT19';

UPDATE refdata.contracts
SET tick_size=1, min_price_ticks=5000, max_price_ticks=15000,
    lower_bound_ticks=5000, upper_bound_ticks=15000,
    multiplier_micro_usdc=100000, updated_at=now()
WHERE ticker IN ('SXF-WTI-26OCT30','SXF-WTIHI-26OCT','SXF-BRENT-26OCT30','SXF-BRENT-26OCT30 (proxy)');

UPDATE refdata.contracts
SET tick_size=1, min_price_ticks=15000, max_price_ticks=40000,
    lower_bound_ticks=15000, upper_bound_ticks=40000,
    multiplier_micro_usdc=100000, updated_at=now()
WHERE ticker IN ('SXF-XAU-26OCT16','SXF-LBMAGOLD-26NOV06');

UPDATE refdata.contracts
SET tick_size=1, min_price_ticks=100, max_price_ticks=130,
    lower_bound_ticks=100, upper_bound_ticks=130,
    multiplier_micro_usdc=100000, updated_at=now()
WHERE ticker='SXF-EURUSD-26OCT16';

UPDATE refdata.contracts
SET tick_size=1, min_price_ticks=0, max_price_ticks=1200,
    lower_bound_ticks=0, upper_bound_ticks=1200,
    multiplier_micro_usdc=1000000, updated_at=now()
WHERE ticker IN ('SXF-HIGHNY-26OCT15','SXF-HIGHDXB-26OCT15','SXF-HIGHTYO-26OCT01');

UPDATE refdata.contracts
SET tick_size=1, min_price_ticks=0, max_price_ticks=1000,
    lower_bound_ticks=0, upper_bound_ticks=1000,
    multiplier_micro_usdc=1000000, updated_at=now()
WHERE ticker IN ('SXF-RAINPHL-26OCT15','SXF-HORMUZW-26OCT18','SXF-HORMUZMA-26DEC31','SXF-OPECP-26DEC','SXF-DELAQI-26NOV09','SXF-CRIC-INDWI-ODI1-RUNS');

UPDATE refdata.contracts
SET tick_size=1, min_price_ticks=150, max_price_ticks=450,
    lower_bound_ticks=150, upper_bound_ticks=450,
    multiplier_micro_usdc=1000000, updated_at=now()
WHERE ticker IN ('SXF-USHOUSE-R-26','SXF-USSENATE-R-26','SXF-UP27-BJP');

UPDATE refdata.contracts
SET tick_size=1, min_price_ticks=0, max_price_ticks=400,
    lower_bound_ticks=0, upper_bound_ticks=400,
    multiplier_micro_usdc=1000000, updated_at=now()
WHERE ticker IN ('SXF-NFLMGN-{GAME}','SXF-EPLGD-{MATCH}','SXF-ATPGAMES-{MATCH}','SXF-SPL-RIYDERBY-GD');

UPDATE refdata.contracts
SET tick_size=1, min_price_ticks=7000, max_price_ticks=9500,
    lower_bound_ticks=7000, upper_bound_ticks=9500,
    multiplier_micro_usdc=10000, updated_at=now()
WHERE ticker='SXF-USDINR-26OCT30';

UPDATE refdata.contracts
SET tick_size=1, min_price_ticks=0, max_price_ticks=1000000,
    lower_bound_ticks=0, upper_bound_ticks=1000000,
    multiplier_micro_usdc=100000, updated_at=now()
WHERE ticker='SXF-HORMUZMA-26DEC31';

