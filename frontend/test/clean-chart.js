// Sarvaex mid price chart, design 3: clean.
// One smooth line, three gridlines, no tooltip box. Hover updates the big number.
//
// const chart = mountCleanChart(el, { getData: async (range) => data, initialRange: '1D' });
// data = {
//   points: [{ t: ms, bid: cents, ask: cents }],     // oldest first
//   fills:  [{ t, price, qty, side }],               // optional, only used for the volume figure
//   events: [{ t, label }],                          // optional, shown as small dots on the line
//   closeLabel: 'Dec 9, 2026'                        // optional
// }
// chart.tick({ bid, ask, fill }) updates the live point; chart.destroy() cleans up.

const CC_CSS = `
.cc{--yes:#2bb3a0;--no:#d0496a;--accent:#6d5ce8;--line:#2a2a30;--muted:#8b8b93;
  background:#16161a;color:#e8e8ea;border:1px solid var(--line);padding:20px 20px 16px;display:flex;flex-direction:column;gap:14px;min-width:0;
  font-family:"Source Serif 4",Georgia,"Times New Roman",serif}
.cc *{box-sizing:border-box}
.cc-top{display:flex;justify-content:space-between;align-items:flex-start;gap:12px}
.cc-lbl{font-size:13px;color:var(--muted);min-height:18px}
.cc-num{display:flex;align-items:baseline;gap:10px;flex-wrap:wrap;margin-top:2px}
.cc-big{font:500 44px/1 "IBM Plex Mono",ui-monospace,monospace;letter-spacing:-.03em;font-variant-numeric:tabular-nums}
.cc-big small{font-size:.45em;color:var(--muted);letter-spacing:0;margin-left:8px;font-family:"Source Serif 4",Georgia,serif}
.cc-chg{font:500 13px "IBM Plex Mono",ui-monospace,monospace}
.cc-chg.up{color:var(--yes)} .cc-chg.down{color:var(--no)} .cc-chg.flat{color:var(--muted)}
.cc-sub{font-size:13px;color:var(--muted)}
.cc-side{display:flex;background:#1e1e23;border:1px solid var(--line);border-radius:999px;padding:3px}
.cc-side button{font:500 13px "Source Serif 4",Georgia,serif;color:var(--muted);background:none;border:0;padding:5px 14px;border-radius:999px;cursor:pointer}
.cc-side button[aria-pressed="true"][data-s="yes"]{background:rgba(43,179,160,.18);color:var(--yes)}
.cc-side button[aria-pressed="true"][data-s="no"]{background:rgba(208,73,106,.18);color:var(--no)}
.cc button:focus-visible,.cc-plot:focus-visible{outline:2px solid var(--accent);outline-offset:3px}
.cc-plot{position:relative;height:var(--cc-h,260px);cursor:crosshair;touch-action:pan-y}
.cc-plot canvas{position:absolute;inset:0;width:100%;height:100%;display:block}
.cc-live{position:absolute;width:8px;height:8px;margin:-4px 0 0 -4px;border-radius:50%;background:var(--c);pointer-events:none}
.cc-live::after{content:"";position:absolute;inset:-7px;border-radius:50%;background:var(--c);opacity:0;animation:cc-ping 2s ease-out infinite}
@keyframes cc-ping{0%{transform:scale(.3);opacity:.45}100%{transform:scale(1.2);opacity:0}}
@media (prefers-reduced-motion:reduce){.cc-live::after{animation:none}}
.cc-empty{position:absolute;inset:0;display:flex;align-items:center;justify-content:center;color:var(--muted);font-size:14px;text-align:center;padding:20px}
.cc-ranges{display:flex;gap:4px}
.cc-ranges button{font:500 12px "IBM Plex Mono",ui-monospace,monospace;color:var(--muted);background:none;border:0;padding:6px 10px;border-radius:6px;cursor:pointer}
.cc-ranges button:hover{color:#fff}
.cc-ranges button[aria-selected="true"]{color:#fff;background:#26262d}
.cc-foot{display:grid;grid-template-columns:repeat(3,minmax(0,1fr));border-top:1px solid var(--line);padding-top:12px;gap:8px}
.cc-foot div{display:flex;flex-direction:column;gap:2px;min-width:0}
.cc-foot span{font-size:12px;color:var(--muted)}
.cc-foot b{font:500 14px "IBM Plex Mono",ui-monospace,monospace;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
`;

const CC_RANGE_TEXT = { '1H': 'past hour', '6H': 'past 6 hours', '1D': 'today', '1W': 'past week', '1M': 'past month', 'ALL': 'all time' };

export function mountCleanChart(root, userOpts = {}) {
  const o = {
    ranges: ['1H', '6H', '1D', '1W', '1M', 'ALL'],
    initialRange: '1D',
    height: 260,
    minPoints: 3,
    colors: { yes: '#2bb3a0', no: '#d0496a', grid: '#24242a', text: '#6f6f78', bg: '#16161a' },
    ...userOpts,
  };
  const C = o.colors;
  if (!document.getElementById('cc-css')) {
    const st = document.createElement('style'); st.id = 'cc-css'; st.textContent = CC_CSS; document.head.appendChild(st);
  }
  root.classList.add('cc');
  root.style.setProperty('--cc-h', o.height + 'px');
  root.innerHTML = `
    <div class="cc-top">
      <div>
        <div class="cc-lbl">Market chance</div>
        <div class="cc-num"><span class="cc-big">–</span><span class="cc-chg"></span><span class="cc-sub"></span></div>
      </div>
      <div class="cc-side" role="group" aria-label="Show price for">
        <button type="button" data-s="yes" aria-pressed="true">Yes</button>
        <button type="button" data-s="no" aria-pressed="false">No</button>
      </div>
    </div>
    <div class="cc-plot" tabindex="0" role="img" aria-label="Price chart. Use left and right arrow keys to inspect.">
      <canvas></canvas><div class="cc-live" hidden></div>
    </div>
    <div class="cc-ranges" role="tablist" aria-label="Time range"></div>
    <div class="cc-foot">
      <div><span>Spread</span><b class="cc-spread">–</b></div>
      <div><span>Volume</span><b class="cc-vol">–</b></div>
      <div><span>Closes</span><b class="cc-close">–</b></div>
    </div>`;

  const $ = (s) => root.querySelector(s);
  const plot = $('.cc-plot'), cvs = plot.querySelector('canvas'), live = $('.cc-live');
  const ctx = cvs.getContext('2d');
  const big = $('.cc-big'), chg = $('.cc-chg'), sub = $('.cc-sub'), lbl = $('.cc-lbl'), tabs = $('.cc-ranges');
  let range = o.initialRange, side = 'yes', data = null, hover = null, geo = null, loadId = 0;

  for (const r of o.ranges) {
    const b = document.createElement('button');
    b.type = 'button'; b.role = 'tab'; b.textContent = r; b.dataset.r = r;
    b.onclick = () => setRange(r);
    tabs.appendChild(b);
  }
  root.querySelectorAll('.cc-side button').forEach((b) => {
    b.onclick = () => {
      side = b.dataset.s;
      root.querySelectorAll('.cc-side button').forEach((x) => x.setAttribute('aria-pressed', String(x === b)));
      header(hover); draw();
    };
  });

  const val = (p) => { const m = (p.bid + p.ask) / 2; return side === 'yes' ? m : 100 - m; };
  const color = () => (side === 'yes' ? C.yes : C.no);
  const fmtN = (v) => (Math.abs(v - Math.round(v)) < 0.05 ? String(Math.round(v)) : v.toFixed(1));
  const fmtUsd = (v) => (v >= 1000 ? '$' + (v / 1000).toFixed(1) + 'K' : '$' + Math.round(v));
  function fmtTime(t) {
    const d = new Date(t);
    const hm = d.toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' });
    const md = d.toLocaleDateString([], { month: 'short', day: 'numeric' });
    return range === '1H' || range === '6H' || range === '1D' ? `${md}, ${hm}` : range === '1W' ? `${md}, ${hm}` : md;
  }

  function header(i) {
    const P = data.points;
    const thin = P.length < o.minPoints;
    const idx = i ?? P.length - 1;
    const v = val(P[idx]), d = v - val(P[0]);
    big.innerHTML = `${fmtN(v)}%<small>${side === 'yes' ? 'Yes' : 'No'}</small>`;
    lbl.textContent = i == null ? 'Market chance' : fmtTime(P[idx].t);
    if (thin) { chg.textContent = ''; sub.textContent = 'indicative, no trades yet'; }
    else {
      chg.className = 'cc-chg ' + (d > 0.05 ? 'up' : d < -0.05 ? 'down' : 'flat');
      chg.textContent = (d > 0.05 ? '▲ ' : d < -0.05 ? '▼ ' : '') + Math.abs(d).toFixed(1) + ' pts';
      const ev = i != null && data.events.find((e) => Math.abs(e.t - P[idx].t) <= geo.tStep * 0.75);
      sub.textContent = i == null ? CC_RANGE_TEXT[range] || '' : ev ? ev.label : `since ${fmtTime(P[0].t)}`;
    }
    const lp = P[P.length - 1];
    $('.cc-spread').textContent = (lp.ask - lp.bid) + '¢';
    $('.cc-vol').textContent = fmtUsd(data.fills.reduce((s, f) => s + (f.qty * f.price) / 100, 0));
    $('.cc-close').textContent = data.closeLabel || '–';
  }

  // monotone cubic path, no overshoot between points
  function smooth(xs, ys) {
    const n = xs.length, dx = [], m = [], t = new Array(n);
    for (let i = 0; i < n - 1; i++) { dx[i] = xs[i + 1] - xs[i]; m[i] = (ys[i + 1] - ys[i]) / (dx[i] || 1); }
    t[0] = m[0]; t[n - 1] = m[n - 2];
    for (let i = 1; i < n - 1; i++) t[i] = m[i - 1] * m[i] <= 0 ? 0 : (m[i - 1] + m[i]) / 2;
    for (let i = 0; i < n - 1; i++) {
      if (m[i] === 0) { t[i] = 0; t[i + 1] = 0; continue; }
      const a = t[i] / m[i], b = t[i + 1] / m[i], s = a * a + b * b;
      if (s > 9) { const k = 3 / Math.sqrt(s); t[i] = k * a * m[i]; t[i + 1] = k * b * m[i]; }
    }
    ctx.moveTo(xs[0], ys[0]);
    for (let i = 0; i < n - 1; i++) {
      const h = dx[i] / 3;
      ctx.bezierCurveTo(xs[i] + h, ys[i] + h * t[i], xs[i + 1] - h, ys[i + 1] - h * t[i + 1], xs[i + 1], ys[i + 1]);
    }
  }

  function draw() {
    if (!data) return;
    const dpr = Math.min(window.devicePixelRatio || 1, 2);
    const w = plot.clientWidth, h = plot.clientHeight;
    cvs.width = Math.round(w * dpr); cvs.height = Math.round(h * dpr);
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, w, h);
    const old = plot.querySelector('.cc-empty'); if (old) old.remove();
    const P = data.points;
    if (P.length < o.minPoints) {
      live.hidden = true; geo = null;
      const e = document.createElement('div'); e.className = 'cc-empty';
      e.textContent = 'Not enough trading yet to draw a chart.';
      plot.appendChild(e);
      return;
    }
    const L = 0, R = 40, T = 14, B = 22;
    const pw = w - L - R, ph = h - T - B;
    const t0 = P[0].t, t1 = P[P.length - 1].t;
    const X = (t) => L + ((t - t0) / (t1 - t0 || 1)) * pw;
    const vs = P.map(val);
    const lo = Math.min(...vs), hi = Math.max(...vs);
    const step = [1, 2, 5, 10, 20, 25].find((s) => (hi - lo + 2) / s <= 3) || 25;
    let yMin = Math.floor((lo - 1) / step) * step, yMax = Math.ceil((hi + 1) / step) * step;
    if (yMax - yMin < 2 * step) yMax = yMin + 2 * step;
    yMin = Math.max(0, yMin); yMax = Math.min(100, yMax);
    const Y = (v) => T + ((yMax - v) / (yMax - yMin)) * ph;
    geo = { X, Y, w, pw, L, tStep: (t1 - t0) / (P.length - 1) };

    // gridlines with labels sitting on them
    ctx.font = '11px "IBM Plex Mono", ui-monospace, monospace';
    for (let v = yMin; v <= yMax + 1e-9; v += step) {
      const y = Math.round(Y(v)) + 0.5;
      ctx.strokeStyle = C.grid; ctx.lineWidth = 1;
      ctx.setLineDash(v === yMin ? [] : [2, 4]);
      ctx.beginPath(); ctx.moveTo(L, y); ctx.lineTo(L + pw, y); ctx.stroke();
      ctx.setLineDash([]);
      ctx.fillStyle = C.text; ctx.textAlign = 'right'; ctx.textBaseline = 'middle';
      ctx.fillText(v + '%', w, y);
    }
    // start and end time only
    ctx.textBaseline = 'alphabetic'; ctx.fillStyle = C.text;
    ctx.textAlign = 'left'; ctx.fillText(fmtTime(t0), L, h - 4);
    ctx.textAlign = 'right'; ctx.fillText('Now', L + pw, h - 4);

    const xs = P.map((p) => X(p.t)), ys = vs.map(Y);
    const col = color();
    const hx = hover != null ? xs[hover] : null;

    // area
    const g = ctx.createLinearGradient(0, T, 0, T + ph);
    g.addColorStop(0, rgbaC(col, 0.16)); g.addColorStop(1, rgbaC(col, 0));
    ctx.beginPath(); smooth(xs, ys); ctx.lineTo(xs[xs.length - 1], T + ph); ctx.lineTo(xs[0], T + ph); ctx.closePath();
    ctx.fillStyle = g; ctx.fill();

    // line: full colour up to the cursor, faded after it
    ctx.lineWidth = 2.25; ctx.lineJoin = 'round'; ctx.lineCap = 'round';
    ctx.save();
    if (hx != null) { ctx.beginPath(); ctx.rect(0, 0, hx, h); ctx.clip(); }
    ctx.beginPath(); smooth(xs, ys); ctx.strokeStyle = col; ctx.stroke();
    ctx.restore();
    if (hx != null) {
      ctx.save(); ctx.beginPath(); ctx.rect(hx, 0, w - hx, h); ctx.clip();
      ctx.beginPath(); smooth(xs, ys); ctx.strokeStyle = rgbaC(col, 0.3); ctx.stroke();
      ctx.restore();
    }
    ctx.lineWidth = 1;

    // event dots
    for (const ev of data.events) {
      if (ev.t < t0 || ev.t > t1) continue;
      let k = 0; while (k < P.length - 1 && P[k + 1].t <= ev.t) k++;
      ctx.beginPath(); ctx.arc(X(ev.t), ys[k], 4, 0, Math.PI * 2);
      ctx.fillStyle = C.bg; ctx.fill(); ctx.lineWidth = 1.5; ctx.strokeStyle = '#c4c4cb'; ctx.stroke(); ctx.lineWidth = 1;
    }

    // hover
    if (hx != null) {
      ctx.strokeStyle = '#3a3a44';
      ctx.beginPath(); ctx.moveTo(Math.round(hx) + 0.5, T - 6); ctx.lineTo(Math.round(hx) + 0.5, T + ph); ctx.stroke();
      ctx.beginPath(); ctx.arc(hx, ys[hover], 5, 0, Math.PI * 2);
      ctx.fillStyle = col; ctx.fill(); ctx.lineWidth = 3; ctx.strokeStyle = C.bg; ctx.stroke(); ctx.lineWidth = 1;
    }

    live.hidden = hover != null;
    live.style.setProperty('--c', col);
    live.style.left = xs[xs.length - 1] + 'px';
    live.style.top = ys[ys.length - 1] + 'px';
  }

  function setHover(i) {
    if (i === hover) return;
    hover = i;
    header(i);
    draw();
  }
  function idxAt(clientX) {
    const r = plot.getBoundingClientRect(), x = clientX - r.left;
    let best = 0, bd = Infinity;
    data.points.forEach((p, i) => { const d = Math.abs(geo.X(p.t) - x); if (d < bd) { bd = d; best = i; } });
    return best;
  }
  plot.addEventListener('pointermove', (e) => { if (geo) setHover(idxAt(e.clientX)); });
  plot.addEventListener('pointerleave', () => setHover(null));
  plot.addEventListener('blur', () => setHover(null));
  plot.addEventListener('keydown', (e) => {
    if (!geo) return;
    const n = data.points.length;
    if (e.key === 'ArrowLeft') { e.preventDefault(); setHover(hover == null ? n - 1 : Math.max(0, hover - 1)); }
    else if (e.key === 'ArrowRight') { e.preventDefault(); setHover(hover == null ? n - 1 : Math.min(n - 1, hover + 1)); }
    else if (e.key === 'Escape') setHover(null);
  });

  async function setRange(r) {
    range = r;
    tabs.querySelectorAll('button').forEach((b) => b.setAttribute('aria-selected', String(b.dataset.r === r)));
    const id = ++loadId;
    const d = await o.getData(r);
    if (id !== loadId) return;
    data = { fills: [], events: [], ...d };
    hover = null;
    draw();
    header(null);
  }

  const ro = new ResizeObserver(draw);
  ro.observe(plot);
  if (document.fonts && document.fonts.ready) document.fonts.ready.then(draw);
  setRange(range);

  return {
    setRange,
    refresh: () => setRange(range),
    tick({ bid, ask, fill } = {}) {
      if (!data || !data.points.length) return;
      const p = data.points[data.points.length - 1];
      if (bid != null) p.bid = bid;
      if (ask != null) p.ask = ask;
      if (fill) data.fills.push(fill);
      draw();
      header(hover);
    },
    destroy() { ro.disconnect(); root.innerHTML = ''; root.classList.remove('cc'); },
  };
}

function rgbaC(hex, a) {
  const n = parseInt(hex.slice(1), 16);
  return `rgba(${(n >> 16) & 255},${(n >> 8) & 255},${n & 255},${a})`;
}
