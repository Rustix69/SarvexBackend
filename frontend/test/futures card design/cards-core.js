// Shared helpers + styles for Sarvaex scalar market cards.

export const CARD_CSS = `
.smg{display:grid;grid-template-columns:repeat(auto-fill,minmax(280px,1fr));gap:12px}
.smc{display:flex;flex-direction:column;padding:20px;background:#18181c;border:1px solid #232328;border-radius:10px;
  color:#e8e8ea;text-decoration:none;min-width:0;transition:border-color .15s;font-family:"Source Serif 4",Georgia,serif}
.smc:hover{border-color:#3a3a44}
.smc:focus-visible{outline:2px solid #6d5ce8;outline-offset:2px}
.smc-top{display:flex;justify-content:space-between;align-items:baseline;gap:12px}
.smc-title{font-size:16px;font-weight:600;line-height:1.3;display:flex;align-items:baseline;gap:8px;min-width:0}
.smc-dot{width:7px;height:7px;border-radius:50%;background:var(--c);flex:none;transform:translateY(-2px)}
.smc-exp{font:12px "IBM Plex Mono",ui-monospace,monospace;color:#6f6f78;white-space:nowrap}
.smc-sub{font-size:13px;color:#7d7d86;margin:4px 0 0 15px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.smc-val{display:flex;align-items:baseline;gap:10px;margin:22px 0 14px}
.smc-val b{font:500 28px/1 "IBM Plex Mono",ui-monospace,monospace;letter-spacing:-.02em;font-variant-numeric:tabular-nums}
.smc-chg{font:12px "IBM Plex Mono",ui-monospace,monospace;color:#6f6f78}
.smc-chg.up{color:#2bb3a0}.smc-chg.down{color:#d0496a}
.smc-track{position:relative;height:4px;border-radius:2px;background:#2a2a31}
.smc-fill{position:absolute;left:0;top:0;bottom:0;border-radius:2px;background:#6d5ce8}
.smc-mark{position:absolute;top:50%;width:10px;height:10px;margin:-5px 0 0 -5px;border-radius:50%;background:#e8e8ea;box-shadow:0 0 0 3px #18181c}
.smc-ends{display:flex;justify-content:space-between;margin-top:8px;font:11px "IBM Plex Mono",ui-monospace,monospace;color:#6f6f78}
`;

export const LIST_CSS = `
.sml{display:flex;flex-direction:column;gap:28px;margin-top:20px;font-family:"Source Serif 4",Georgia,serif;color:#e8e8ea}
.sml-group h3{display:flex;align-items:center;gap:8px;margin:0 0 6px;font-size:14px;font-weight:600;color:#c4c4cb}
.sml-group h3 i{width:7px;height:7px;border-radius:50%;background:var(--c)}
.sml-group h3 span{font:400 12px "IBM Plex Mono",ui-monospace,monospace;color:#6f6f78}
.sml-head,.sml-row{display:grid;grid-template-columns:minmax(0,1fr) 110px 90px minmax(160px,240px) 70px;align-items:center;gap:20px;padding:0 14px}
.sml-head{font:11px "IBM Plex Mono",ui-monospace,monospace;color:#5f5f68;padding-bottom:8px;border-bottom:1px solid #232328}
.sml-head span:nth-child(2),.sml-head span:nth-child(3),.sml-head span:nth-child(5){text-align:right}
.sml-row{min-height:64px;border-bottom:1px solid #1f1f24;color:inherit;text-decoration:none;border-radius:6px;transition:background .12s}
.sml-row:hover{background:#18181c}
.sml-row:focus-visible{outline:2px solid #6d5ce8;outline-offset:-2px}
.sml-name{min-width:0}
.sml-name b{display:block;font-size:15px;font-weight:600;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.sml-name small{display:block;font-size:12.5px;color:#7d7d86;margin-top:2px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.sml-val{font:500 17px "IBM Plex Mono",ui-monospace,monospace;text-align:right;font-variant-numeric:tabular-nums;white-space:nowrap}
.sml-chg{font:12px "IBM Plex Mono",ui-monospace,monospace;text-align:right;color:#6f6f78;white-space:nowrap}
.sml-chg.up{color:#2bb3a0}.sml-chg.down{color:#d0496a}
.sml-range{display:grid;grid-template-columns:60px 1fr 60px;align-items:center;gap:10px;font:11px "IBM Plex Mono",ui-monospace,monospace;color:#6f6f78}
.sml-range span:first-child{text-align:right}
.sml-bar{position:relative;height:4px;border-radius:2px;background:#2a2a31}
.sml-bar i{position:absolute;top:50%;width:8px;height:8px;margin:-4px 0 0 -4px;border-radius:50%;background:#e8e8ea;box-shadow:0 0 0 3px #111114}
.sml-bar u{position:absolute;left:0;top:0;bottom:0;border-radius:2px;background:#6d5ce8;text-decoration:none}
.sml-row:hover .sml-bar i{box-shadow:0 0 0 3px #18181c}
.sml-exp{font:12px "IBM Plex Mono",ui-monospace,monospace;color:#8b8b93;text-align:right;white-space:nowrap}
.sml-exp.soon{color:#e6a23c}
@media (max-width:760px){
  .sml-range{grid-template-columns:auto 1fr auto}
  .sml-head{display:none}
  .sml-row{grid-template-columns:1fr auto;grid-template-areas:"name exp" "val chg" "range range";gap:6px 12px;padding:14px}
  .sml-name{grid-area:name}.sml-exp{grid-area:exp;align-self:start}
  .sml-val{grid-area:val;text-align:left}.sml-chg{grid-area:chg}
  .sml-range{grid-area:range}
}
`;

export const CATEGORIES = {
  rates:     { label: 'Rates',     color: '#8b7ff0' },
  inflation: { label: 'Inflation', color: '#e6a23c' },
  jobs:      { label: 'Jobs',      color: '#5b94d6' },
  equities:  { label: 'Equities',  color: '#2bb3a0' },
  crypto:    { label: 'Crypto',    color: '#d9c04a' },
  energy:    { label: 'Energy',    color: '#d0496a' },
  fx:        { label: 'FX',        color: '#4fb6d6' },
  Economics: { label: 'Economics', color: '#8b7ff0' },
  Finance: { label: 'Finance', color: '#2bb3a0' },
  Commodities: { label: 'Commodities', color: '#d0496a' },
  Elections: { label: 'Elections', color: '#c26a9a' },
  Climate: { label: 'Climate', color: '#5b94d6' },
  'Geopolitics / Shipping': { label: 'Geopolitics / Shipping', color: '#4fb6d6' },
  Other: { label: 'Other', color: '#8b8894' },
};

// value formatting per unit
export function fmt(unit, v, short = false) {
  switch (unit) {
    case 'pct': return v.toFixed(2) + '%';
    case 'usd': return '$' + v.toFixed(2);
    case 'usdK': return '$' + (short ? Math.round(v / 1000) : (v / 1000).toFixed(1)) + 'K';
    case 'index': return Math.round(v).toLocaleString('en-US');
    case 'fx': return v.toFixed(4);
    case 'jobsK': return (v > 0 ? '+' : v < 0 ? '−' : '') + Math.abs(Math.round(v)) + 'K';
    default: return String(v);
  }
}
export function fmtChange(unit, from, to) {
  const d = to - from;
  const dir = Math.abs(d) < 1e-9 ? 'flat' : d > 0 ? 'up' : 'down';
  const arrow = dir === 'up' ? '▲ ' : dir === 'down' ? '▼ ' : '';
  let txt;
  if (unit === 'pct') txt = Math.abs(d).toFixed(2) + ' pts';
  else if (unit === 'jobsK') txt = Math.abs(Math.round(d)) + 'K';
  else txt = Math.abs((d / from) * 100).toFixed(1) + '%';
  return { dir, text: arrow + txt };
}
export function settleText(settle) {
  if (settle === 'recurring') return { text: 'Hourly', soon: true };
  const ms = new Date(settle).getTime() - Date.now();
  const days = Math.ceil(ms / 864e5);
  if (days <= 0) return { text: 'Settling', soon: true };
  if (days === 1) return { text: 'Tomorrow', soon: true };
  if (days <= 7) return { text: `in ${days}d`, soon: true };
  return { text: new Date(settle).toLocaleDateString('en-US', { month: 'short', day: 'numeric' }), soon: false };
}
export function sparkPath(history, w = 96, h = 36) {
  const lo = Math.min(...history), hi = Math.max(...history), span = hi - lo || 1;
  return history.map((v, i) => `${i ? 'L' : 'M'}${((i / (history.length - 1)) * w).toFixed(1)},${(h - 3 - ((v - lo) / span) * (h - 6)).toFixed(1)}`).join('');
}
const esc = (t) => String(t).replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' })[c]);

export function cardHTML(m) {
  const cat = CATEGORIES[m.category];
  const pos = Math.max(0, Math.min(100, ((m.value - m.min) / (m.max - m.min)) * 100));
  const chg = fmtChange(m.unit, m.history[0], m.value);
  const st = settleText(m.settle);
  return `
  <a class="smc" href="${esc(m.href || '#')}" title="${esc(m.source)} · settles ${esc(st.text)}"
     aria-label="${esc(m.title)}, ${esc(m.subtitle)}. Estimate ${fmt(m.unit, m.value)}, range ${fmt(m.unit, m.min, true)} to ${fmt(m.unit, m.max, true)}. Settles ${esc(st.text)}.">
    <div class="smc-top">
      <div class="smc-title"><span class="smc-dot" style="--c:${cat.color}" aria-hidden="true"></span><span>${esc(m.title)}</span></div>
      <span class="smc-exp">${esc(st.text)}</span>
    </div>
    <div class="smc-sub">${esc(m.subtitle)}</div>
    <div class="smc-val"><b>${fmt(m.unit, m.value)}</b><span class="smc-chg ${chg.dir}">${chg.text}</span></div>
    <div class="smc-track"><div class="smc-fill" style="width:${pos}%"></div><div class="smc-mark" style="left:${pos}%"></div></div>
    <div class="smc-ends"><span>${fmt(m.unit, m.min, true)}</span><span>${fmt(m.unit, m.max, true)}</span></div>
  </a>`;
}

export function groupByCategory(markets) {
  const order = Object.keys(CATEGORIES);
  const groups = {};
  for (const m of markets) (groups[m.category] ||= []).push(m);
  return order.filter((k) => groups[k]).map((k) => ({ key: k, ...CATEGORIES[k], markets: groups[k] }));
}

export function listHTML(markets) {
  const head = '<div class="sml-head" aria-hidden="true"><span>Market</span><span>Estimate</span><span>7d</span><span>Range</span><span>Settles</span></div>';
  return '<div class="sml">' + groupByCategory(markets).map((g, gi) => `
    <section class="sml-group" aria-label="${g.label}">
      <h3><i style="--c:${g.color}"></i>${g.label}<span>${g.markets.length}</span></h3>
      ${gi === 0 ? head : ''}
      ${g.markets.map((m) => {
        const pos = Math.max(0, Math.min(100, ((m.value - m.min) / (m.max - m.min)) * 100));
        const chg = fmtChange(m.unit, m.history[0], m.value);
        const st = settleText(m.settle);
        return `<a class="sml-row" href="${esc(m.href || '#')}" title="${esc(m.source)}">
          <div class="sml-name"><b>${esc(m.title)}</b><small>${esc(m.subtitle)}</small></div>
          <div class="sml-val">${fmt(m.unit, m.value)}</div>
          <div class="sml-chg ${chg.dir}">${chg.text}</div>
          <div class="sml-range"><span>${fmt(m.unit, m.min, true)}</span><div class="sml-bar"><u style="width:${pos}%"></u><i style="left:${pos}%"></i></div><span>${fmt(m.unit, m.max, true)}</span></div>
          <div class="sml-exp${st.soon ? ' soon' : ''}">${esc(st.text)}</div>
        </a>`;
      }).join('')}
    </section>`).join('') + '</div>';
}
