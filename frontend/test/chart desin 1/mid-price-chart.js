// Sarvaex mid price chart.
// const chart = mountMidPriceChart(el, { getData: async (range) => data, initialRange: '1D' });
// data = {
//   points: [{ t: ms, bid: cents, ask: cents }],          // order-book snapshots, oldest first
//   fills:  [{ t: ms, price: cents, qty: shares, side: 'yes' | 'no' }],
//   events: [{ t: ms, label: 'CPI release' }],             // optional
//   last:   cents,                                         // last traded price (optional)
//   closeLabel: 'Closes Dec 9, 2026'                       // optional
// }
// chart.tick({ bid, ask, fill })  updates the live point; chart.destroy() cleans up.

const MPC_CSS = `
.mpc{--yes:#2bb3a0;--no:#d0496a;--accent:#6d5ce8;--mpc-bg:#16161a;--line:#2a2a30;--muted:#8b8b93;
  background:var(--mpc-bg);color:#e8e8ea;border:1px solid var(--line);padding:16px 16px 12px;
  display:flex;flex-direction:column;gap:12px;min-width:0;
  font-family:"Source Serif 4",Georgia,"Times New Roman",serif}
.mpc *{box-sizing:border-box}
.mpc-mono{font-family:"IBM Plex Mono",ui-monospace,Menlo,Consolas,monospace}
.mpc-head{display:flex;justify-content:space-between;align-items:flex-start;gap:14px;flex-wrap:wrap}
.mpc-label{font-size:13px;color:var(--muted)}
.mpc-value{display:flex;align-items:baseline;gap:8px;margin-top:2px;flex-wrap:wrap}
.mpc-big{font:500 34px/1.1 "IBM Plex Mono",ui-monospace,monospace;font-variant-numeric:tabular-nums;letter-spacing:-.02em}
.mpc-unit{font-size:15px;color:var(--muted)}
.mpc-chg{font:500 13px "IBM Plex Mono",ui-monospace,monospace;margin-left:4px}
.mpc-chg.up{color:var(--yes)} .mpc-chg.down{color:var(--no)} .mpc-chg.flat{color:var(--muted)}
.mpc-period{font-size:13px;color:var(--muted)}
.mpc-stats{display:flex;gap:6px 16px;flex-wrap:wrap;margin-top:8px;font:12px "IBM Plex Mono",ui-monospace,monospace;color:var(--muted)}
.mpc-stats b{color:#e8e8ea;font-weight:500}
.mpc-ranges{display:flex;background:#1e1e23;border:1px solid var(--line);border-radius:8px;padding:3px}
.mpc-ranges button{font:500 12px "IBM Plex Mono",ui-monospace,monospace;color:var(--muted);background:none;border:0;padding:6px 10px;border-radius:6px;cursor:pointer}
.mpc-ranges button:hover{color:#fff}
.mpc-ranges button[aria-selected="true"]{background:#2e2e36;color:#fff}
.mpc-legend{display:flex;gap:6px;flex-wrap:wrap}
.mpc-legend .it{display:inline-flex;align-items:center;gap:7px;font-size:12.5px;color:#c4c4cb;background:none;border:1px solid var(--line);border-radius:999px;padding:4px 11px;font-family:inherit}
.mpc-legend button{cursor:pointer}
.mpc-legend button:hover{border-color:#44444d}
.mpc-legend button[aria-pressed="false"]{opacity:.4}
.mpc button:focus-visible,.mpc-plot:focus-visible{outline:2px solid var(--accent);outline-offset:2px}
.mpc-sw{display:inline-block;width:14px;height:2px;border-radius:2px}
.mpc-sw.mid{background:var(--yes);height:3px}
.mpc-sw.band{height:10px;background:rgba(43,179,160,.22);border-top:1px solid rgba(43,179,160,.5);border-bottom:1px solid rgba(43,179,160,.5);border-radius:1px}
.mpc-sw.fill{width:9px;height:9px;border-radius:50%;background:linear-gradient(90deg,var(--yes) 50%,var(--no) 50%)}
.mpc-sw.ev{width:0;height:12px;border-left:1px dashed var(--muted);border-radius:0}
.mpc-plot{position:relative;height:var(--mpc-h,300px);border-radius:4px}
.mpc-plot canvas,.mpc-volwrap canvas{position:absolute;inset:0;width:100%;height:100%;display:block}
.mpc-volwrap{position:relative;height:56px;border-top:1px solid #222228}
.mpc-live{position:absolute;width:10px;height:10px;margin:-5px 0 0 -5px;border-radius:50%;background:var(--yes);pointer-events:none}
.mpc-live::after{content:"";position:absolute;inset:-6px;border-radius:50%;border:2px solid var(--yes);opacity:0;animation:mpc-ping 1.8s ease-out infinite}
@keyframes mpc-ping{0%{transform:scale(.4);opacity:.8}100%{transform:scale(1.4);opacity:0}}
@media (prefers-reduced-motion:reduce){.mpc-live::after{animation:none}}
.mpc-tip{position:absolute;top:26px;pointer-events:none;background:#0e0e11;border:1px solid #30303a;border-radius:8px;padding:8px 10px;
  font:12px/1.6 "IBM Plex Mono",ui-monospace,monospace;color:#e8e8ea;min-width:170px;box-shadow:0 8px 24px rgba(0,0,0,.45);z-index:2}
.mpc-tip .t{color:var(--muted);margin-bottom:2px}
.mpc-tip .r{display:flex;justify-content:space-between;gap:14px}
.mpc-tip .y{color:var(--yes)} .mpc-tip .n{color:var(--no)}
.mpc-empty{position:absolute;inset:0;display:flex;align-items:center;justify-content:center;color:var(--muted);font-size:14px;text-align:center;padding:20px}
.mpc-foot{display:flex;justify-content:space-between;gap:10px;flex-wrap:wrap;font-size:12px;color:#6f6f78}
`;

const RANGE_TEXT = { '1H': 'past hour', '6H': 'past 6 hours', '1D': 'past day', '1W': 'past week', '1M': 'past month', 'ALL': 'since listing' };

export function mountMidPriceChart(root, userOpts = {}) {
  const o = {
    ranges: ['1H', '6H', '1D', '1W', '1M', 'ALL'],
    initialRange: '1D',
    height: 300,
    minPoints: 3,
    colors: { yes: '#2bb3a0', no: '#d0496a', accent: '#6d5ce8', bg: '#16161a', grid: '#232329', text: '#8b8b93', ref: '#5a3b4b' },
    ...userOpts,
  };
  const C = o.colors;
  if (!document.getElementById('mpc-css')) {
    const st = document.createElement('style'); st.id = 'mpc-css'; st.textContent = MPC_CSS; document.head.appendChild(st);
  }
  root.classList.add('mpc');
  root.style.setProperty('--mpc-h', o.height + 'px');
  root.innerHTML = `
    <div class="mpc-head">
      <div>
        <div class="mpc-label">Mid price</div>
        <div class="mpc-value"><span class="mpc-big">–</span><span class="mpc-unit">chance</span><span class="mpc-chg"></span><span class="mpc-period"></span></div>
        <div class="mpc-stats"></div>
      </div>
      <div class="mpc-ranges" role="tablist" aria-label="Time range"></div>
    </div>
    <div class="mpc-legend">
      <span class="it"><i class="mpc-sw mid"></i>Mid</span>
      <button type="button" class="it" data-k="band" aria-pressed="true"><i class="mpc-sw band"></i>Bid / ask</button>
      <button type="button" class="it" data-k="fills" aria-pressed="true"><i class="mpc-sw fill"></i>Fills</button>
      <button type="button" class="it" data-k="events" aria-pressed="true"><i class="mpc-sw ev"></i>Events</button>
    </div>
    <div class="mpc-plot" tabindex="0" role="img" aria-label="Mid price chart. Use left and right arrow keys to inspect points.">
      <canvas></canvas><div class="mpc-live" hidden></div><div class="mpc-tip" hidden></div>
    </div>
    <div class="mpc-volwrap"><canvas></canvas></div>
    <div class="mpc-foot"><span class="mpc-close"></span><span>Powered by Sarvaex</span></div>`;

  const $ = (s) => root.querySelector(s);
  const plot = $('.mpc-plot'), cMain = plot.querySelector('canvas'), live = $('.mpc-live'), tip = $('.mpc-tip');
  const volWrap = $('.mpc-volwrap'), cVol = volWrap.querySelector('canvas');
  const big = $('.mpc-big'), chg = $('.mpc-chg'), period = $('.mpc-period'), stats = $('.mpc-stats'), label = $('.mpc-label');
  const ctx = cMain.getContext('2d'), vctx = cVol.getContext('2d');

  let range = o.initialRange, data = null, hover = null, geo = null, loadId = 0;
  const show = { band: true, fills: true, events: true };

  // range tabs
  const tabs = $('.mpc-ranges');
  for (const r of o.ranges) {
    const b = document.createElement('button');
    b.type = 'button'; b.role = 'tab'; b.textContent = r; b.dataset.r = r;
    b.onclick = () => setRange(r);
    tabs.appendChild(b);
  }
  root.querySelectorAll('.mpc-legend button').forEach((b) => {
    b.onclick = () => { show[b.dataset.k] = !show[b.dataset.k]; b.setAttribute('aria-pressed', String(show[b.dataset.k])); draw(); };
  });

  const mid = (p) => (p.bid + p.ask) / 2;
  const fmtC = (v) => (Number.isInteger(v) ? v : v.toFixed(1)) + '¢';
  const fmtP = (v) => (Number.isInteger(v) ? v : v.toFixed(1)) + '%';
  const fmtUsd = (v) => (v >= 1000 ? '$' + (v / 1000).toFixed(1) + 'K' : '$' + Math.round(v));
  function fmtTime(t, full) {
    const d = new Date(t);
    const hm = d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
    const md = d.toLocaleDateString([], { month: 'short', day: 'numeric' });
    if (full) return `${md}, ${hm}`;
    if (range === '1H' || range === '6H' || range === '1D') return hm;
    if (range === '1W') return d.toLocaleDateString([], { weekday: 'short', day: 'numeric' });
    return md;
  }

  function sizeCanvas(c, g) {
    const dpr = Math.min(window.devicePixelRatio || 1, 2);
    const w = c.clientWidth, h = c.clientHeight;
    c.width = Math.round(w * dpr); c.height = Math.round(h * dpr);
    g.setTransform(dpr, 0, 0, dpr, 0, 0);
    return { w, h };
  }

  function header(idx) {
    const pts = data.points;
    const i = idx ?? pts.length - 1;
    const v = mid(pts[i]), first = mid(pts[0]);
    const d = v - first;
    big.textContent = fmtP(v);
    const thin = pts.length < o.minPoints;
    chg.className = 'mpc-chg ' + (d > 0 ? 'up' : d < 0 ? 'down' : 'flat');
    chg.textContent = thin ? '' : (d > 0 ? '▲ ' : d < 0 ? '▼ ' : '') + Math.abs(d).toFixed(1) + ' pts';
    period.textContent = thin ? 'indicative, from quotes only' : idx == null ? RANGE_TEXT[range] || '' : 'since ' + fmtTime(pts[0].t, true);
    label.textContent = idx == null ? 'Mid price' : fmtTime(pts[i].t, true);
    const lp = pts[pts.length - 1];
    const vol = data.fills.reduce((s, f) => s + (f.qty * f.price) / 100, 0);
    stats.innerHTML =
      `<span>Bid <b>${fmtC(lp.bid)}</b></span><span>Ask <b>${fmtC(lp.ask)}</b></span>` +
      `<span>Spread <b>${fmtC(lp.ask - lp.bid)}</b></span>` +
      (data.last != null ? `<span>Last <b>${fmtC(data.last)}</b></span>` : '') +
      `<span>Vol <b>${fmtUsd(vol)}</b></span><span>Fills <b>${data.fills.length}</b></span>`;
  }

  function niceStep(span) {
    const raw = span / 5;
    for (const s of [0.5, 1, 2, 5, 10, 20, 25]) if (s >= raw) return s;
    return 25;
  }

  function draw() {
    if (!data) return;
    const { w, h } = sizeCanvas(cMain, ctx);
    const vs = sizeCanvas(cVol, vctx);
    ctx.clearRect(0, 0, w, h); vctx.clearRect(0, 0, vs.w, vs.h);
    const pts = data.points;
    const old = plot.querySelector('.mpc-empty'); if (old) old.remove();
    if (pts.length < o.minPoints) {
      live.hidden = true;
      const e = document.createElement('div');
      e.className = 'mpc-empty';
      e.textContent = 'Not enough trading in this range yet. Try a longer range or place the first order.';
      plot.appendChild(e);
      return;
    }
    const L = 6, R = 58, T = 24, B = 24;
    const pw = w - L - R, ph = h - T - B;
    const t0 = pts[0].t, t1 = pts[pts.length - 1].t;
    const X = (t) => L + ((t - t0) / (t1 - t0 || 1)) * pw;

    let lo = Infinity, hi = -Infinity;
    for (const p of pts) {
      const a = show.band ? p.bid : mid(p), b = show.band ? p.ask : mid(p);
      if (a < lo) lo = a; if (b > hi) hi = b;
    }
    if (show.fills) for (const f of data.fills) { if (f.price < lo) lo = f.price; if (f.price > hi) hi = f.price; }
    const step = niceStep(Math.max(4, hi - lo));
    let yMin = Math.floor((lo - step * 0.3) / step) * step, yMax = Math.ceil((hi + step * 0.3) / step) * step;
    yMin = Math.max(0, yMin); yMax = Math.min(100, yMax);
    const Y = (v) => T + ((yMax - v) / (yMax - yMin)) * ph;
    geo = { L, R, T, B, pw, ph, X, Y, w, h };

    // grid + right axis
    ctx.font = '11px "IBM Plex Mono", ui-monospace, monospace';
    ctx.textBaseline = 'middle';
    const tagY = Y(mid(pts[pts.length - 1]));
    for (let v = yMin; v <= yMax + 1e-9; v += step) {
      const y = Math.round(Y(v)) + 0.5;
      const hideLabel = Math.abs(y - tagY) < 14;
      ctx.strokeStyle = C.grid; ctx.lineWidth = 1; ctx.setLineDash([]);
      ctx.beginPath(); ctx.moveTo(L, y); ctx.lineTo(L + pw, y); ctx.stroke();
      ctx.fillStyle = C.text; ctx.textAlign = 'left';
      if (!hideLabel) ctx.fillText(fmtC(v), L + pw + 10, y);
    }
    // 50% reference
    if (50 > yMin && 50 < yMax) {
      const y = Math.round(Y(50)) + 0.5;
      ctx.strokeStyle = C.ref; ctx.setLineDash([4, 4]);
      ctx.beginPath(); ctx.moveTo(L, y); ctx.lineTo(L + pw, y); ctx.stroke();
      ctx.setLineDash([]);
      ctx.fillStyle = C.no; ctx.globalAlpha = 0.7; ctx.textAlign = 'left';
      ctx.fillText('50/50', L + 4, y - 8); ctx.globalAlpha = 1;
    }
    // x axis labels
    ctx.textBaseline = 'alphabetic'; ctx.fillStyle = C.text;
    const nx = Math.max(2, Math.min(6, Math.floor(pw / 110)));
    for (let i = 0; i <= nx; i++) {
      const t = t0 + ((t1 - t0) * i) / nx;
      ctx.textAlign = i === 0 ? 'left' : i === nx ? 'right' : 'center';
      ctx.fillText(fmtTime(t), X(t), h - 6);
    }
    // events
    if (show.events && data.events) {
      for (const ev of data.events) {
        if (ev.t < t0 || ev.t > t1) continue;
        const x = Math.round(X(ev.t)) + 0.5;
        ctx.strokeStyle = '#4a4a55'; ctx.setLineDash([3, 3]);
        ctx.beginPath(); ctx.moveTo(x, T - 4); ctx.lineTo(x, T + ph); ctx.stroke();
        ctx.setLineDash([]);
        ctx.font = '12px "Source Serif 4", Georgia, serif';
        const tw = ctx.measureText(ev.label).width + 12;
        const bx = Math.min(Math.max(L, x - tw / 2), L + pw - tw);
        ctx.fillStyle = '#23232a'; roundRect(ctx, bx, 2, tw, 18, 4); ctx.fill();
        ctx.fillStyle = '#c4c4cb'; ctx.textAlign = 'left'; ctx.textBaseline = 'middle';
        ctx.fillText(ev.label, bx + 6, 11.5);
        ctx.font = '11px "IBM Plex Mono", ui-monospace, monospace';
      }
    }
    // step path helper
    const stepPath = (g, get) => {
      g.moveTo(X(pts[0].t), Y(get(pts[0])));
      for (let i = 1; i < pts.length; i++) { g.lineTo(X(pts[i].t), Y(get(pts[i - 1]))); g.lineTo(X(pts[i].t), Y(get(pts[i]))); }
    };
    // bid/ask band
    if (show.band) {
      ctx.beginPath();
      stepPath(ctx, (p) => p.ask);
      for (let i = pts.length - 1; i >= 1; i--) { ctx.lineTo(X(pts[i].t), Y(pts[i].bid)); ctx.lineTo(X(pts[i].t), Y(pts[i - 1].bid)); }
      ctx.lineTo(X(pts[0].t), Y(pts[0].bid));
      ctx.closePath();
      ctx.fillStyle = hexA(C.yes, 0.1); ctx.fill();
    }
    // area under mid
    const grad = ctx.createLinearGradient(0, T, 0, T + ph);
    grad.addColorStop(0, hexA(C.yes, 0.22)); grad.addColorStop(1, hexA(C.yes, 0));
    ctx.beginPath(); stepPath(ctx, mid);
    ctx.lineTo(X(t1), T + ph); ctx.lineTo(X(t0), T + ph); ctx.closePath();
    ctx.fillStyle = grad; ctx.fill();
    // mid line
    ctx.beginPath(); stepPath(ctx, mid);
    ctx.strokeStyle = C.yes; ctx.lineWidth = 2; ctx.lineJoin = 'round'; ctx.stroke();
    ctx.lineWidth = 1;
    // fills
    if (show.fills) {
      for (const f of data.fills) {
        const r = 2 + Math.min(4.5, Math.sqrt(f.qty) * 0.38);
        ctx.beginPath(); ctx.arc(X(f.t), Y(f.price), r, 0, Math.PI * 2);
        ctx.fillStyle = hexA(f.side === 'yes' ? C.yes : C.no, 0.6); ctx.fill();
        ctx.strokeStyle = C.bg; ctx.stroke();
      }
    }
    // live price tag
    const lastMid = mid(pts[pts.length - 1]);
    const ly = Y(lastMid);
    ctx.strokeStyle = hexA(C.accent, 0.6); ctx.setLineDash([2, 3]);
    ctx.beginPath(); ctx.moveTo(L, Math.round(ly) + 0.5); ctx.lineTo(L + pw, Math.round(ly) + 0.5); ctx.stroke();
    ctx.setLineDash([]);
    ctx.fillStyle = C.accent; roundRect(ctx, L + pw + 4, ly - 10, R - 6, 20, 4); ctx.fill();
    ctx.fillStyle = '#fff'; ctx.textAlign = 'center'; ctx.textBaseline = 'middle';
    ctx.font = '500 11px "IBM Plex Mono", ui-monospace, monospace';
    ctx.fillText(fmtP(lastMid), L + pw + 4 + (R - 6) / 2, ly + 0.5);
    live.hidden = hover != null;
    live.style.left = X(t1) + 'px'; live.style.top = ly + 'px';

    // volume bars
    const vol = new Array(pts.length).fill(0), yes = new Array(pts.length).fill(0);
    let fi = 0;
    const fills = [...data.fills].sort((a, b) => a.t - b.t);
    for (let i = 0; i < pts.length; i++) {
      const end = i + 1 < pts.length ? pts[i + 1].t : Infinity;
      while (fi < fills.length && fills[fi].t < end) {
        const f = fills[fi++]; const usd = (f.qty * f.price) / 100;
        if (f.t >= pts[i].t || i === 0) { vol[i] += usd; if (f.side === 'yes') yes[i] += usd; }
      }
    }
    const vmax = Math.max(1, ...vol);
    const bw = Math.max(1, (pw / pts.length) * 0.7);
    for (let i = 0; i < pts.length; i++) {
      if (!vol[i]) continue;
      const bh = (vol[i] / vmax) * (vs.h - 14);
      const x = X(pts[i].t) + (pw / pts.length) * 0.15;
      vctx.globalAlpha = hover === i ? 0.95 : 0.5;
      vctx.fillStyle = yes[i] >= vol[i] / 2 ? C.yes : C.no;
      vctx.fillRect(Math.min(x, L + pw - bw), vs.h - bh, bw, bh);
    }
    vctx.globalAlpha = 1;
    vctx.font = '11px "IBM Plex Mono", ui-monospace, monospace';
    vctx.fillStyle = C.text; vctx.textBaseline = 'top'; vctx.textAlign = 'left';
    vctx.fillText('Volume', L, 4);
    vctx.fillText(fmtUsd(vmax), L + pw + 10, 4);
    geo.vol = vol;

    // hover crosshair
    if (hover != null) {
      const p = pts[hover], x = Math.round(X(p.t)) + 0.5, y = Y(mid(p));
      ctx.strokeStyle = '#6a6a75'; ctx.setLineDash([]);
      ctx.beginPath(); ctx.moveTo(x, T); ctx.lineTo(x, T + ph); ctx.stroke();
      vctx.strokeStyle = '#6a6a75'; vctx.beginPath(); vctx.moveTo(x, 0); vctx.lineTo(x, vs.h); vctx.stroke();
      ctx.beginPath(); ctx.arc(x, y, 5, 0, Math.PI * 2);
      ctx.fillStyle = C.bg; ctx.fill(); ctx.lineWidth = 2; ctx.strokeStyle = C.yes; ctx.stroke(); ctx.lineWidth = 1;
    }
  }

  function showTip(i) {
    const p = data.points[i];
    const end = i + 1 < data.points.length ? data.points[i + 1].t : Infinity;
    const fs = data.fills.filter((f) => f.t >= p.t && f.t < end);
    const ys = fs.filter((f) => f.side === 'yes').reduce((s, f) => s + f.qty, 0);
    const ns = fs.filter((f) => f.side === 'no').reduce((s, f) => s + f.qty, 0);
    tip.innerHTML =
      `<div class="t">${fmtTime(p.t, true)}</div>` +
      `<div class="r"><span>Mid</span><b>${fmtP(mid(p))}</b></div>` +
      `<div class="r"><span>Bid / Ask</span><span>${fmtC(p.bid)} / ${fmtC(p.ask)}</span></div>` +
      (fs.length ? `<div class="r"><span>Fills</span><span><span class="y">${ys} Yes</span> · <span class="n">${ns} No</span></span></div>` : `<div class="r"><span>Fills</span><span>none</span></div>`) +
      `<div class="r"><span>Volume</span><span>${fmtUsd(geo.vol[i] || 0)}</span></div>`;
    tip.hidden = false;
    const x = geo.X(p.t);
    const tw = tip.offsetWidth;
    tip.style.left = (x + 14 + tw > geo.w ? x - 14 - tw : x + 14) + 'px';
  }
  function setHover(i) {
    hover = i;
    if (i == null) { tip.hidden = true; header(null); }
    else { header(i); }
    draw();
    if (i != null) showTip(i);
  }
  function idxAt(clientX) {
    const r = plot.getBoundingClientRect();
    const x = clientX - r.left;
    const pts = data.points;
    let best = 0, bd = Infinity;
    for (let i = 0; i < pts.length; i++) { const d = Math.abs(geo.X(pts[i].t) - x); if (d < bd) { bd = d; best = i; } }
    return best;
  }
  const onMove = (e) => { if (geo && data.points.length >= o.minPoints) setHover(idxAt(e.clientX)); };
  const onLeave = () => setHover(null);
  const onKey = (e) => {
    if (!data || data.points.length < o.minPoints) return;
    const n = data.points.length;
    if (e.key === 'ArrowLeft') { e.preventDefault(); setHover(hover == null ? n - 1 : Math.max(0, hover - 1)); }
    else if (e.key === 'ArrowRight') { e.preventDefault(); setHover(hover == null ? n - 1 : Math.min(n - 1, hover + 1)); }
    else if (e.key === 'Escape') setHover(null);
  };
  plot.addEventListener('pointermove', onMove);
  plot.addEventListener('pointerleave', onLeave);
  plot.addEventListener('keydown', onKey);
  plot.addEventListener('blur', onLeave);

  async function setRange(r) {
    range = r;
    tabs.querySelectorAll('button').forEach((b) => b.setAttribute('aria-selected', String(b.dataset.r === r)));
    const id = ++loadId;
    const d = await o.getData(r);
    if (id !== loadId) return;
    data = { fills: [], events: [], ...d };
    $('.mpc-close').textContent = data.closeLabel || '';
    hover = null; tip.hidden = true;
    header(null);
    draw();
  }

  const ro = new ResizeObserver(() => { draw(); if (hover != null) showTip(hover); });
  ro.observe(plot);
  if (document.fonts && document.fonts.ready) document.fonts.ready.then(draw);
  setRange(range);

  return {
    setRange,
    refresh: () => setRange(range),
    tick({ bid, ask, fill, last } = {}) {
      if (!data || !data.points.length) return;
      const p = data.points[data.points.length - 1];
      if (bid != null) p.bid = bid;
      if (ask != null) p.ask = ask;
      if (fill) data.fills.push(fill);
      if (last != null) data.last = last;
      if (hover == null) header(null);
      draw();
    },
    destroy() { ro.disconnect(); root.innerHTML = ''; root.classList.remove('mpc'); },
  };
}

function roundRect(g, x, y, w, h, r) {
  g.beginPath();
  g.moveTo(x + r, y); g.arcTo(x + w, y, x + w, y + h, r); g.arcTo(x + w, y + h, x, y + h, r);
  g.arcTo(x, y + h, x, y, r); g.arcTo(x, y, x + w, y, r); g.closePath();
}
function hexA(hex, a) {
  const n = parseInt(hex.slice(1), 16);
  return `rgba(${(n >> 16) & 255},${(n >> 8) & 255},${n & 255},${a})`;
}
