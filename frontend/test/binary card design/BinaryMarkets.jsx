'use client';
import { BINARY_CSS, EVENT_CSS, EVENT_CARD_CSS, binaryView, groupByEvent } from './binary-core';
import { CATEGORIES, settleText } from './cards-core';
import { groupByCategory } from './cards-core';

// market = { id, question, category, yes, change, yesAsk, noAsk, settle, href, traded }
// onBuy(market, 'yes' | 'no') opens your trade ticket.

const cents = (v) => (v == null ? '' : `${Math.round(v)}¢`);

function BuyButtons({ m, onBuy }) {
  return (
    <>
      <button type="button" className="bm-btn yes" onClick={() => onBuy?.(m, 'yes')}
              aria-label={`Buy Yes${m.yesAsk != null ? ` at ${cents(m.yesAsk)}` : ''}`}>
        Yes<span>{cents(m.yesAsk)}</span>
      </button>
      <button type="button" className="bm-btn no" onClick={() => onBuy?.(m, 'no')}
              aria-label={`Buy No${m.noAsk != null ? ` at ${cents(m.noAsk)}` : ''}`}>
        No<span>{cents(m.noAsk)}</span>
      </button>
    </>
  );
}

export function BinaryCard({ market: m, onBuy, onOpen }) {
  const v = binaryView(m);
  return (
    <article className="bmc" onClick={(event) => { if (onOpen && !event.target.closest('button')) onOpen(m); }}>
      <div className="bmc-top">
        <span className="bmc-cat"><i style={{ '--c': v.cat.color }} />{v.cat.label}</span>
        <span className={v.st.soon ? 'bm-soon' : ''}>{v.st.text}</span>
      </div>
      <a className="bmc-q" href={m.href || '#'} title={m.question} onClick={(event) => { if (onOpen) { event.preventDefault(); onOpen(m); } }}>{m.question}</a>
      <div className="bmc-odds">
        <b>{v.odds}</b><span className="bm-lbl">{v.label}</span>
        <span className={`bm-chg ${v.chgDir}`}>{v.chgText}</span>
      </div>
      <div className={`bm-bar${v.traded ? '' : ' empty'}`} aria-hidden="true"><i style={{ width: `${v.width}%` }} /></div>
      <div className="bmc-btns"><BuyButtons m={m} onBuy={onBuy} /></div>
    </article>
  );
}

export function BinaryGrid({ markets, onBuy }) {
  return (
    <>
      <style>{BINARY_CSS}</style>
      <div className="bmg">{markets.map((m) => <BinaryCard key={m.id} market={m} onBuy={onBuy} />)}</div>
    </>
  );
}

export function BinaryList({ markets, onBuy }) {
  return (
    <>
      <style>{BINARY_CSS}</style>
      <div className="bml">
        {groupByCategory(markets).map((g, gi) => (
          <section key={g.key} aria-label={g.label}>
            <h3><i style={{ '--c': g.color }} />{g.label}<span>{g.markets.length}</span></h3>
            {gi === 0 && (
              <div className="bml-head" aria-hidden="true">
                <span>Market</span><span>Chance</span><span>7d</span><span>Buy</span><span>Settles</span>
              </div>
            )}
            {g.markets.map((m) => {
              const v = binaryView(m);
              return (
                <div key={m.id} className="bml-row">
                  <a className="bml-q" href={m.href || '#'} title={m.question}>{m.question}</a>
                  <div className="bml-odds">
                    {v.traded ? (
                      <><b>{v.odds}</b><div className="bm-bar" aria-hidden="true"><i style={{ width: `${v.width}%` }} /></div></>
                    ) : <span className="bm-lbl">No trades yet</span>}
                  </div>
                  <span className={`bm-chg ${v.chgDir}`}>{v.chgText}</span>
                  <div className="bml-btns"><BuyButtons m={m} onBuy={onBuy} /></div>
                  <span className={`bml-exp${v.st.soon ? ' bm-soon' : ''}`}>{v.st.text}</span>
                </div>
              );
            })}
          </section>
        ))}
      </div>
    </>
  );
}

// events = [{ id, title, subtitle, category }]
// markets additionally need eventId, outcome (short label, e.g. "Hike exactly 25 bp") and detail (e.g. "27–28 Oct meeting")
export function EventCards({ markets, events, onBuy }) {
  return (
    <>
      <style>{BINARY_CSS + EVENT_CSS}</style>
      <div className="evg">
        {groupByEvent(markets, events).map((e) => {
          const cat = CATEGORIES[e.category];
          const dates = e.markets.filter((m) => m.settle !== 'recurring').map((m) => new Date(m.settle).getTime());
          const nx = settleText(dates.length ? new Date(Math.min(...dates)).toISOString().slice(0, 10) : 'recurring');
          return (
            <article key={e.id} className="evc">
              <div className="evc-head">
                <div>
                  <h3><i style={{ '--c': cat.color }} />{e.title}</h3>
                  {e.subtitle && <p>{e.subtitle}</p>}
                </div>
                <span className="evc-next">{e.markets.length > 1 ? 'Next ' : ''}{nx.text}</span>
              </div>
              {e.markets.map((m) => {
                const v = binaryView(m);
                return (
                  <div key={m.id} className="evr" style={{ '--p': `${v.width}%` }}>
                    <div className="evr-txt">
                      <a href={m.href || '#'} title={m.question}>{m.outcome}</a>
                      <small>
                        {m.detail}{m.detail ? ' · ' : ''}
                        <span className={v.st.soon ? 'bm-soon' : ''}>{v.st.text}</span>
                      </small>
                    </div>
                    <div className="evr-act">
                      <div className="evr-odds">
                        {v.traded
                          ? <><span className={`bm-chg ${v.chgDir}`}>{v.chgText}</span><b>{v.odds}</b></>
                          : <span className="muted">No trades yet</span>}
                      </div>
                      <div className="evr-btns"><BuyButtons m={m} onBuy={onBuy} /></div>
                    </div>
                  </div>
                );
              })}
            </article>
          );
        })}
      </div>
    </>
  );
}

// One card per market, labelled with its event.
// events = [{ id, title, category }]; markets need eventId, outcome and (optional) detail.
export function EventCard({ market: m, event, onBuy }) {
  const v = binaryView(m);
  const color = CATEGORIES[event?.category || m.category].color;
  return (
    <article className="ecd">
      <div className="ecd-top">
        <span className="ecd-ev"><i style={{ '--c': color }} /><span>{event?.title || v.cat.label}</span></span>
        <span className={v.st.soon ? 'bm-soon' : ''}>{v.st.text}</span>
      </div>
      <div className="ecd-body">
        <a className="ecd-title" href={m.href || '#'} title={m.question}>{m.outcome}</a>
        {m.detail && <div className="ecd-detail">{m.detail}</div>}
      </div>
      <div className={`ecd-row${v.traded ? '' : ' empty'}`} style={{ '--p': `${v.width}%` }}>
        <div className="ecd-odds">
          {v.traded
            ? <><b>{v.odds}</b><span className="bm-lbl">chance</span><span className={`bm-chg ${v.chgDir}`}>{v.chgText}</span></>
            : <span className="bm-lbl">No trades yet</span>}
        </div>
        <div className="ecd-act">
          <span className="ecd-hint">{v.traded ? 'Trade' : 'Place first order'} →</span>
          <div className="ecd-btns"><BuyButtons m={m} onBuy={onBuy} /></div>
        </div>
      </div>
    </article>
  );
}

export function EventCardGrid({ markets, events, onBuy }) {
  const byId = Object.fromEntries(events.map((e) => [e.id, e]));
  return (
    <>
      <style>{BINARY_CSS + EVENT_CARD_CSS}</style>
      <div className="ecg">
        {markets.map((m) => <EventCard key={m.id} market={m} event={byId[m.eventId]} onBuy={onBuy} />)}
      </div>
    </>
  );
}
