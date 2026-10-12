-- OPTIONAL: align each future's settlement_source text with its binary's (same agency, wording only).
-- Edits contract terms: confirm with product/legal before applying. See backfill_report.md.
BEGIN;
UPDATE refdata.contracts f SET settlement_source = b.settlement_source FROM refdata.contracts b WHERE f.event_ticker = b.event_ticker AND f.kind = 'SCALAR' AND b.kind = 'BINARY' AND f.event_ticker = 'XEV-STD-FIN-04';
UPDATE refdata.contracts f SET settlement_source = b.settlement_source FROM refdata.contracts b WHERE f.event_ticker = b.event_ticker AND f.kind = 'SCALAR' AND b.kind = 'BINARY' AND f.event_ticker = 'XEV-REG-AS-02';
UPDATE refdata.contracts f SET settlement_source = b.settlement_source FROM refdata.contracts b WHERE f.event_ticker = b.event_ticker AND f.kind = 'SCALAR' AND b.kind = 'BINARY' AND f.event_ticker = 'XEV-STD-COM-01';
UPDATE refdata.contracts f SET settlement_source = b.settlement_source FROM refdata.contracts b WHERE f.event_ticker = b.event_ticker AND f.kind = 'SCALAR' AND b.kind = 'BINARY' AND f.event_ticker = 'XEV-REG-ME-01';
UPDATE refdata.contracts f SET settlement_source = b.settlement_source FROM refdata.contracts b WHERE f.event_ticker = b.event_ticker AND f.kind = 'SCALAR' AND b.kind = 'BINARY' AND f.event_ticker = 'XEV-STD-ECO-05';
UPDATE refdata.contracts f SET settlement_source = b.settlement_source FROM refdata.contracts b WHERE f.event_ticker = b.event_ticker AND f.kind = 'SCALAR' AND b.kind = 'BINARY' AND f.event_ticker = 'XEV-STD-FIN-05';
UPDATE refdata.contracts f SET settlement_source = b.settlement_source FROM refdata.contracts b WHERE f.event_ticker = b.event_ticker AND f.kind = 'SCALAR' AND b.kind = 'BINARY' AND f.event_ticker = 'XEV-REG-IND-06';
UPDATE refdata.contracts f SET settlement_source = b.settlement_source FROM refdata.contracts b WHERE f.event_ticker = b.event_ticker AND f.kind = 'SCALAR' AND b.kind = 'BINARY' AND f.event_ticker = 'XEV-REG-ME-04';
UPDATE refdata.contracts f SET settlement_source = b.settlement_source FROM refdata.contracts b WHERE f.event_ticker = b.event_ticker AND f.kind = 'SCALAR' AND b.kind = 'BINARY' AND f.event_ticker = 'XEV-STD-GEO-01';
UPDATE refdata.contracts f SET settlement_source = b.settlement_source FROM refdata.contracts b WHERE f.event_ticker = b.event_ticker AND f.kind = 'SCALAR' AND b.kind = 'BINARY' AND f.event_ticker = 'XEV-REG-AS-04';
COMMIT;
