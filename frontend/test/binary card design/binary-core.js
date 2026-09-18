// Sarvaex binary (Yes/No) market cards and list rows.
// market = { id, question, category, yes, change, yesAsk, noAsk, settle, source, href, traded }
//   yes: implied chance 0–100 (e.g. mid or last price); change: points over 7d
//   yesAsk / noAsk: cents to buy each side (null if no offers); traded: false shows the empty state
import { CATEGORIES, settleText, groupByCategory } from './cards-core';

export const BINARY_CSS = `
.bmg{display:grid;grid-template-columns:repeat(auto-fill,minmax(280px,1fr));gap:12px}
.bmc{display:flex;flex-direction:column;padding:18px;background:#18181c;border:1px solid #232328;border-radius:10px;color:#e8e8ea;min-width:0;
  font-family:"Source Serif 4",Georgia,serif;transition:border-color .15s}
.bmc:hover{border-color:#3a3a44}
.bmc-top{display:flex;justify-content:space-between;align-items:center;gap:8px;font:12px "IBM Plex Mono",ui-monospace,monospace;color:#6f6f78}
.bmc-cat{display:inline-flex;align-items:center;gap:7px}
.bmc-cat i{width:7px;height:7px;border-radius:50%;background:var(--c)}
.bm-soon{color:#e6a23c}
.bmc-q{margin-top:10px;font-size:15.5px;font-weight:600;line-height:1.35;color:inherit;text-decoration:none;
  display:-webkit-box;-webkit-line-clamp:3;-webkit-box-orient:vertical;overflow:hidden;min-height:4.05em}
.bmc-q:hover{text-decoration:underline;text-decoration-color:#4a4a55;text-underline-offset:3px}
.bmc-q:focus-visible,.bm-btn:focus-visible,.bml-q:focus-visible{outline:2px solid #6d5ce8;outline-offset:2px;border-radius:4px}
.bmc-odds{display:flex;align-items:baseline;gap:8px;margin:18px 0 10px}
.bmc-odds b{font:500 28px/1 "IBM Plex Mono",ui-monospace,monospace;letter-spacing:-.02em}
.bm-lbl{font-size:13px;color:#8b8b93}
.bm-chg{font:12px "IBM Plex Mono",ui-monospace,monospace;color:#6f6f78;white-space:nowrap}
.bm-chg.up{color:#2bb3a0}.bm-chg.down{color:#d0496a}
.bm-bar{height:4px;border-radius:2px;background:rgba(208,73,106,.45);overflow:hidden}
.bm-bar i{display:block;height:100%;background:#2bb3a0;border-radius:2px 0 0 2px}
.bm-bar.empty{background:#2a2a31}
.bmc-btns{display:grid;grid-template-columns:1fr 1fr;gap:8px;margin-top:16px}
.bm-btn{display:flex;justify-content:space-between;align-items:center;gap:6px;padding:9px 12px;border-radius:7px;border:0;cursor:pointer;
  font:500 13px "Source Serif 4",Georgia,serif;transition:background .12s}
.bm-btn span{font-family:"IBM Plex Mono",ui-monospace,monospace;font-size:12.5px}
.bm-btn.yes{background:rgba(43,179,160,.12);color:#3cc4b0}
.bm-btn.yes:hover{background:rgba(43,179,160,.24)}
.bm-btn.no{background:rgba(208,73,106,.12);color:#e0607f}
.bm-btn.no:hover{background:rgba(208,73,106,.24)}

.bml{display:flex;flex-direction:column;gap:28px;margin-top:20px;font-family:"Source Serif 4",Georgia,serif;color:#e8e8ea}
.bml h3{display:flex;align-items:center;gap:8px;margin:0 0 6px;font-size:14px;font-weight:600;color:#c4c4cb}
.bml h3 i{width:7px;height:7px;border-radius:50%;background:var(--c)}
.bml h3 span{font:400 12px "IBM Plex Mono",ui-monospace,monospace;color:#6f6f78}
.bml-head,.bml-row{display:grid;grid-template-columns:minmax(0,1fr) 150px 60px 190px 64px;align-items:center;gap:20px;padding:0 14px}
.bml-head{font:11px "IBM Plex Mono",ui-monospace,monospace;color:#5f5f68;padding-bottom:8px;border-bottom:1px solid #232328}
.bml-head span:nth-child(3),.bml-head span:nth-child(5){text-align:right}
.bml-row{min-height:62px;border-bottom:1px solid #1f1f24;border-radius:6px;transition:background .12s}
.bml-row:hover{background:#18181c}
.bml-q{color:inherit;text-decoration:none;font-size:14.5px;font-weight:600;line-height:1.35;display:-webkit-box;-webkit-line-clamp:2;-webkit-box-orient:vertical;overflow:hidden}
.bml-odds{display:grid;grid-template-columns:44px 1fr;align-items:center;gap:10px}
.bml-odds b{font:500 15px "IBM Plex Mono",ui-monospace,monospace;text-align:right}
.bml-odds .bm-lbl{grid-column:1/-1;font-size:12px}
.bml-row .bm-chg{text-align:right}
.bml-btns{display:grid;grid-template-columns:1fr 1fr;gap:6px}
.bml-btns .bm-btn{padding:7px 10px}
.bml-exp{font:12px "IBM Plex Mono",ui-monospace,monospace;color:#8b8b93;text-align:right;white-space:nowrap}
@media (max-width:760px){
  .bml-head{display:none}
  .bml-q{-webkit-line-clamp:3}
  .bml-row{grid-template-columns:1fr auto;grid-template-areas:"q exp" "odds chg" "btns btns";gap:10px 12px;padding:14px}
  .bml-q{grid-area:q}.bml-exp{grid-area:exp;align-self:start}
  .bml-odds{grid-area:odds}.bml-row .bm-chg{grid-area:chg}.bml-btns{grid-area:btns}
}
`;

export const EVENT_CSS = `
.evg{display:grid;grid-template-columns:repeat(auto-fill,minmax(360px,1fr));gap:12px;align-items:start}
.evc{background:#18181c;border:1px solid #232328;border-radius:10px;padding:16px 8px 8px;color:#e8e8ea;font-family:"Source Serif 4",Georgia,serif;min-width:0}
.evc-head{display:flex;justify-content:space-between;align-items:baseline;gap:10px;padding:0 10px 12px}
.evc-head h3{margin:0;font-size:16px;font-weight:600;display:flex;align-items:center;gap:8px}
.evc-head h3 i{width:7px;height:7px;border-radius:50%;background:var(--c);flex:none}
.evc-head p{margin:3px 0 0 15px;font-size:12.5px;color:#7d7d86}
.evc-next{font:12px "IBM Plex Mono",ui-monospace,monospace;color:#6f6f78;white-space:nowrap}
.evr{position:relative;display:flex;align-items:center;justify-content:space-between;gap:12px;padding:10px;border-radius:7px;
  background:linear-gradient(90deg,rgba(43,179,160,.09) var(--p),transparent var(--p))}
.evr + .evr{margin-top:4px}
.evr:hover,.evr:focus-within{background:linear-gradient(90deg,rgba(43,179,160,.16) var(--p),#1e1e24 var(--p))}
.evr-txt{min-width:0}
.evr-txt a{display:block;color:inherit;text-decoration:none;font-size:14.5px;font-weight:600;line-height:1.3}
.evr-txt a:hover{text-decoration:underline;text-decoration-color:#4a4a55;text-underline-offset:3px}
.evr-txt small{display:block;margin-top:2px;font-size:12px;color:#7d7d86}
.evr-txt small .bm-soon{font-family:"IBM Plex Mono",ui-monospace,monospace}
.evr-act{display:grid;flex:none;min-width:132px;justify-items:end}
.evr-odds,.evr-btns{grid-area:1/1;transition:opacity .12s}
.evr-odds{display:flex;align-items:baseline;gap:8px}
.evr-odds b{font:500 18px "IBM Plex Mono",ui-monospace,monospace}
.evr-odds .muted{font-size:12px;color:#7d7d86}
.evr-btns{display:flex;gap:6px;opacity:0;pointer-events:none}
.evr-btns .bm-btn{padding:6px 9px;gap:8px;font-size:12.5px}
.evr:hover .evr-odds,.evr:focus-within .evr-odds{opacity:0}
.evr:hover .evr-btns,.evr:focus-within .evr-btns{opacity:1;pointer-events:auto}
.evr-txt a:focus-visible{outline:2px solid #6d5ce8;outline-offset:2px;border-radius:3px}
@media (hover:none){
  .evr{flex-wrap:wrap}
  .evr-act{min-width:0;width:100%;display:flex;justify-content:space-between;align-items:center}
  .evr-btns{opacity:1;pointer-events:auto}
  .evr:hover .evr-odds,.evr:focus-within .evr-odds{opacity:1}
}
@media (max-width:420px){.evg{grid-template-columns:1fr}}
`;

export const EVENT_CARD_CSS = `
.ecg{display:grid;grid-template-columns:repeat(auto-fill,minmax(290px,1fr));gap:12px}
.ecd{display:flex;flex-direction:column;background:#18181c;border:1px solid #232328;border-radius:10px;padding:16px;color:#e8e8ea;
  font-family:"Source Serif 4",Georgia,serif;min-width:0;transition:border-color .15s}
.ecd:hover,.ecd:focus-within{border-color:#3a3a44}
.ecd-top{display:flex;justify-content:space-between;align-items:center;gap:10px;font:12px "IBM Plex Mono",ui-monospace,monospace;color:#6f6f78}
.ecd-ev{display:inline-flex;align-items:center;gap:7px;min-width:0}
.ecd-ev i{width:7px;height:7px;border-radius:50%;background:var(--c);flex:none}
.ecd-ev span{white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.ecd-body{flex:1;padding:14px 0 18px}
.ecd-title{display:block;font-size:19px;font-weight:600;line-height:1.25;color:inherit;text-decoration:none}
.ecd-title:hover{text-decoration:underline;text-decoration-color:#4a4a55;text-underline-offset:3px}
.ecd-title:focus-visible{outline:2px solid #6d5ce8;outline-offset:2px;border-radius:3px}
.ecd-detail{margin-top:4px;font-size:13px;color:#7d7d86}
.ecd-row{display:flex;align-items:center;justify-content:space-between;gap:10px;min-height:46px;padding:0 12px;border-radius:8px;
  background:linear-gradient(90deg,rgba(43,179,160,.13) var(--p),#1f1f25 var(--p))}
.ecd-row.empty{background:#1f1f25}
.ecd-odds{display:flex;align-items:baseline;gap:8px;white-space:nowrap}
.ecd-odds b{font:500 20px "IBM Plex Mono",ui-monospace,monospace}
.ecd-act{display:grid;justify-items:end;align-items:center}
.ecd-hint,.ecd-btns{grid-area:1/1;transition:opacity .12s}
.ecd-hint{font-size:12.5px;color:#7d7d86;white-space:nowrap}
.ecd-btns{display:flex;gap:6px;opacity:0;pointer-events:none}
.ecd-btns .bm-btn{padding:6px 9px;gap:8px;font-size:12.5px}
.ecd:hover .ecd-hint,.ecd:focus-within .ecd-hint{opacity:0}
.ecd:hover .ecd-btns,.ecd:focus-within .ecd-btns{opacity:1;pointer-events:auto}
@media (hover:none){
  .ecd-hint{display:none}
  .ecd-btns{opacity:1;pointer-events:auto}
}
`;

const escB = (t) => String(t).replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' })[c]);
const cents = (v) => (v == null ? '' : Math.round(v) + '¢');

function categoryKey(m) {
  const raw = String(m.category || '').trim();
  const identity = `${m.id || ''} ${m.question || ''}`.toLowerCase();
  if (raw.toLowerCase().startsWith('crypto') || /\b(crypto|bitcoin|btc|ethereum|eth)\b/.test(identity)) return 'Crypto';
  return raw;
}

export function binaryView(m) {
  const traded = m.traded !== false;
  const d = m.change || 0;
  return {
    traded,
    odds: traded ? Math.round(m.yes) + '%' : '–',
    label: traded ? 'chance' : 'No trades yet',
    chgDir: !traded || Math.abs(d) < 0.5 ? 'flat' : d > 0 ? 'up' : 'down',
    chgText: !traded || Math.abs(d) < 0.5 ? '' : (d > 0 ? '▲ ' : '▼ ') + Math.round(Math.abs(d)),
    width: traded ? Math.max(0, Math.min(100, m.yes)) : 0,
    st: settleText(m.settle),
    cat: CATEGORIES[categoryKey(m)] || CATEGORIES.Other,
  };
}

const buttons = (m) =>
  `<button type="button" class="bm-btn yes" data-id="${escB(m.id)}" data-side="yes" aria-label="Buy Yes${m.yesAsk != null ? ' at ' + cents(m.yesAsk) : ''}">Yes<span>${cents(m.yesAsk)}</span></button>` +
  `<button type="button" class="bm-btn no" data-id="${escB(m.id)}" data-side="no" aria-label="Buy No${m.noAsk != null ? ' at ' + cents(m.noAsk) : ''}">No<span>${cents(m.noAsk)}</span></button>`;

export function binaryCardHTML(m) {
  const v = binaryView(m);
  return `
  <article class="bmc">
    <div class="bmc-top">
      <span class="bmc-cat"><i style="--c:${v.cat.color}"></i>${v.cat.label}</span>
      <span class="${v.st.soon ? 'bm-soon' : ''}">${escB(v.st.text)}</span>
    </div>
    <a class="bmc-q" href="${escB(m.href || '#')}" title="${escB(m.question)}">${escB(m.question)}</a>
    <div class="bmc-odds"><b>${v.odds}</b><span class="bm-lbl">${v.label}</span><span class="bm-chg ${v.chgDir}">${v.chgText}</span></div>
    <div class="bm-bar${v.traded ? '' : ' empty'}" aria-hidden="true"><i style="width:${v.width}%"></i></div>
    <div class="bmc-btns">${buttons(m)}</div>
  </article>`;
}

export function binaryListHTML(markets) {
  const head = '<div class="bml-head" aria-hidden="true"><span>Market</span><span>Chance</span><span>7d</span><span>Buy</span><span>Settles</span></div>';
  return '<div class="bml">' + groupByCategory(markets).map((g, gi) => `
    <section aria-label="${g.label}">
      <h3><i style="--c:${g.color}"></i>${g.label}<span>${g.markets.length}</span></h3>
      ${gi === 0 ? head : ''}
      ${g.markets.map((m) => {
        const v = binaryView(m);
        return `<div class="bml-row">
          <a class="bml-q" href="${escB(m.href || '#')}" title="${escB(m.question)}">${escB(m.question)}</a>
          <div class="bml-odds">${v.traded
            ? `<b>${v.odds}</b><div class="bm-bar" aria-hidden="true"><i style="width:${v.width}%"></i></div>`
            : `<span class="bm-lbl">No trades yet</span>`}</div>
          <span class="bm-chg ${v.chgDir}">${v.chgText}</span>
          <div class="bml-btns">${buttons(m)}</div>
          <span class="bml-exp${v.st.soon ? ' bm-soon' : ''}">${escB(v.st.text)}</span>
        </div>`;
      }).join('')}
    </section>`).join('') + '</div>';
}

// events = [{ id, title, subtitle, category }]; each market has eventId, outcome (short label) and detail (small line)
export function groupByEvent(markets, events) {
  return events
    .map((e) => ({ ...e, markets: markets.filter((m) => m.eventId === e.id) }))
    .filter((e) => e.markets.length);
}
function nextSettle(markets) {
  const ts = markets.filter((m) => m.settle !== 'recurring').map((m) => new Date(m.settle).getTime());
  return ts.length ? settleText(new Date(Math.min(...ts)).toISOString().slice(0, 10)) : settleText('recurring');
}
export function eventCardHTML(e) {
  const cat = CATEGORIES[e.category];
  const nx = nextSettle(e.markets);
  return `
  <article class="evc">
    <div class="evc-head">
      <div><h3><i style="--c:${cat.color}"></i>${escB(e.title)}</h3>${e.subtitle ? `<p>${escB(e.subtitle)}</p>` : ''}</div>
      <span class="evc-next">${e.markets.length > 1 ? 'Next ' : ''}${escB(nx.text)}</span>
    </div>
    ${e.markets.map((m) => {
      const v = binaryView(m);
      return `<div class="evr" style="--p:${v.width}%">
        <div class="evr-txt">
          <a href="${escB(m.href || '#')}" title="${escB(m.question)}">${escB(m.outcome)}</a>
          <small>${escB(m.detail || '')}${m.detail ? ' · ' : ''}<span class="${v.st.soon ? 'bm-soon' : ''}">${escB(v.st.text)}</span></small>
        </div>
        <div class="evr-act">
          <div class="evr-odds">${v.traded ? `<span class="bm-chg ${v.chgDir}">${v.chgText}</span><b>${v.odds}</b>` : '<span class="muted">No trades yet</span>'}</div>
          <div class="evr-btns">${buttons(m)}</div>
        </div>
      </div>`;
    }).join('')}
  </article>`;
}

// One card per market. eventsById = { fed: { title, category }, ... }
export function eventCardSingleHTML(m, eventsById) {
  const v = binaryView(m);
  const ev = eventsById[m.eventId] || { title: v.cat.label, category: m.category };
  const color = CATEGORIES[ev.category || m.category].color;
  return `
  <article class="ecd">
    <div class="ecd-top">
      <span class="ecd-ev"><i style="--c:${color}"></i><span>${escB(ev.title)}</span></span>
      <span class="${v.st.soon ? 'bm-soon' : ''}">${escB(v.st.text)}</span>
    </div>
    <div class="ecd-body">
      <a class="ecd-title" href="${escB(m.href || '#')}" title="${escB(m.question)}">${escB(m.outcome)}</a>
      ${m.detail ? `<div class="ecd-detail">${escB(m.detail)}</div>` : ''}
    </div>
    <div class="ecd-row${v.traded ? '' : ' empty'}" style="--p:${v.width}%">
      <div class="ecd-odds">${v.traded
        ? `<b>${v.odds}</b><span class="bm-lbl">chance</span><span class="bm-chg ${v.chgDir}">${v.chgText}</span>`
        : '<span class="bm-lbl">No trades yet</span>'}</div>
      <div class="ecd-act">
        <span class="ecd-hint">${v.traded ? 'Trade' : 'Place first order'} →</span>
        <div class="ecd-btns">${buttons(m)}</div>
      </div>
    </div>
  </article>`;
}
