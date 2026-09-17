'use client';
import { LIST_CSS, fmt, fmtChange, settleText, groupByCategory } from './cards-core';

// Same market objects as ScalarMarketCards.jsx.
export default function ScalarMarketList({ markets }) {
  return (
    <>
      <style>{LIST_CSS}</style>
      <div className="sml">
        {groupByCategory(markets).map((g, gi) => (
          <section key={g.key} className="sml-group" aria-label={g.label}>
            <h3><i style={{ '--c': g.color }} />{g.label}<span>{g.markets.length}</span></h3>
            {gi === 0 && (
              <div className="sml-head" aria-hidden="true">
                <span>Market</span><span>Estimate</span><span>7d</span><span>Range</span><span>Settles</span>
              </div>
            )}
            {g.markets.map((m) => {
              const pos = Math.max(0, Math.min(100, ((m.value - m.min) / (m.max - m.min)) * 100));
              const chg = fmtChange(m.unit, m.history[0], m.value);
              const st = settleText(m.settle);
              return (
                <a key={m.id} className="sml-row" href={m.href || '#'} title={m.source}>
                  <div className="sml-name"><b>{m.title}</b><small>{m.subtitle}</small></div>
                  <div className="sml-val">{fmt(m.unit, m.value)}</div>
                  <div className={`sml-chg ${chg.dir}`}>{chg.text}</div>
                  <div className="sml-range">
                    <span>{fmt(m.unit, m.min, true)}</span>
                    <div className="sml-bar"><u style={{ width: `${pos}%` }} /><i style={{ left: `${pos}%` }} /></div>
                    <span>{fmt(m.unit, m.max, true)}</span>
                  </div>
                  <div className={`sml-exp${st.soon ? ' soon' : ''}`}>{st.text}</div>
                </a>
              );
            })}
          </section>
        ))}
      </div>
    </>
  );
}
