-- Demo-data decision: both legs of the four mismatched pairs share one expiry.
UPDATE refdata.contracts SET expected_resolution_at='2026-10-31 23:59:00+00', updated_at=now() WHERE event_ticker IN ('XEV-REG-EU-02','XEV-STD-ECO-04','XEV-REG-IND-02');
UPDATE refdata.contracts SET expected_resolution_at='2026-11-01 23:59:00+00', updated_at=now() WHERE event_ticker='XEV-STD-ECO-03';
