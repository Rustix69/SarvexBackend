// Sarvaex backdrop, design C: dot-matrix market board.
// Borrows common prediction-market patterns: a big "chance" number, a stepped
// probability chart, Yes/No prices in cents and a live order book ladder.
// Usage: const bd = mountMatrixBackdrop(element, { ...options }); later bd.destroy();

const MX_DEFAULTS = {
  pitch: 8,                // grid spacing, px
  dot: 3,                  // dot size, px
  step: 350,               // ms per chart column (lower = faster)
  tickMs: 90,
  center: 56,
  volatility: 0.3,
  minRange: 16,
  market: 'FOMC +25BP  OCT 27-28',
  chartTop: 0.6,           // chart band starts this far down the panel (0–1)
  panelMinWidth: 700,      // below this width only the chart is drawn
  fontFamily: '"IBM Plex Mono", ui-monospace, Menlo, Consolas, monospace',
  colors: {
    bg: '#0c0d11', grid: '#16181e', up: '#37a57a', down: '#d0496a',
    text: '#7d808a', bright: '#e9eaee', accent: '#e8588f',
  },
};

const GLYPHS = {
  '0': ['01110', '10001', '10011', '10101', '11001', '10001', '01110'],
  '1': ['00100', '01100', '00100', '00100', '00100', '00100', '01110'],
  '2': ['01110', '10001', '00001', '00010', '00100', '01000', '11111'],
  '3': ['11111', '00010', '00100', '00010', '00001', '10001', '01110'],
  '4': ['00010', '00110', '01010', '10010', '11111', '00010', '00010'],
  '5': ['11111', '10000', '11110', '00001', '00001', '10001', '01110'],
  '6': ['00110', '01000', '10000', '11110', '10001', '10001', '01110'],
  '7': ['11111', '00001', '00010', '00100', '01000', '01000', '01000'],
  '8': ['01110', '10001', '10001', '01110', '10001', '10001', '01110'],
  '9': ['01110', '10001', '10001', '01111', '00001', '00010', '01100'],
  '%': ['11000', '11001', '00010', '00100', '01000', '10011', '00011'],
};

export function mountMatrixBackdrop(container, userOpts = {}) {
  const o = { ...MX_DEFAULTS, ...userOpts, colors: { ...MX_DEFAULTS.colors, ...(userOpts.colors || {}) } };
  const C = o.colors;
  const P = o.pitch, D = o.dot, OFF = (P - D) / 2;
  const reduced = typeof matchMedia !== 'undefined' && matchMedia('(prefers-reduced-motion: reduce)').matches;
  const speed = reduced ? 0.25 : 1;

  const cvs = document.createElement('canvas');
  cvs.setAttribute('aria-hidden', 'true');
  Object.assign(cvs.style, { position: 'absolute', inset: '0', width: '100%', height: '100%', display: 'block', pointerEvents: 'none' });
  container.prepend(cvs);
  const ctx = cvs.getContext('2d');
  const grid = document.createElement('canvas');
  const gctx = grid.getContext('2d');
  const FONT = `400 11px ${o.fontFamily}`;
  const FONT_B = `500 12px ${o.fontFamily}`;

  let W = 0, H = 0, dpr = 1, running = true, visible = true, raf = 0, last = 0, t = 0;
  let stepAcc = 0, tickAcc = 0, drift = 0, v = o.center, sMin = null, sMax = null;
  const hist = [];
  const book = new Map(); // price -> { size, flash }
  const gauss = () => (Math.random() + Math.random() + Math.random() - 1.5) / 0.5;
  const clamp = (x, a, b) => Math.max(a, Math.min(b, x));

  function tickPrice() {
    if (Math.random() < 0.012) drift = (Math.random() * 2 - 1) * 0.07;
    v += (o.center - v) * 0.006 + drift + gauss() * o.volatility;
    if (Math.random() < 0.002) v += (Math.random() < 0.5 ? -1 : 1) * (3 + Math.random() * 4);
    v = clamp(v, 3, 97);
  }
  for (let i = 0; i < 400; i++) { for (let k = 0; k < o.step / o.tickMs; k++) tickPrice(); hist.push(v); }

  const level = (price) => {
    if (!book.has(price)) {
      const dist = Math.abs(price - Math.round(v));
      book.set(price, { size: (200 + Math.random() * 1300) * (1 + dist * 0.35), flash: 0 });
    }
    return book.get(price);
  };
  function tickBook() {
    const mid = Math.round(v);
    for (const [p, L] of book) {
      if (Math.abs(p - mid) > 12) { book.delete(p); continue; }
      L.size = clamp(L.size * (1 + gauss() * 0.035), 60, 9000);
      L.flash = Math.max(0, L.flash - 0.08);
    }
    if (Math.random() < 0.09) {
      const L = level(Math.random() < 0.5 ? mid : mid + 1);
      L.flash = 1;
      L.size = Math.max(60, L.size * (0.6 + Math.random() * 0.3));
    }
  }

  function buildGrid() {
    grid.width = Math.round(W * dpr); grid.height = Math.round(H * dpr);
    gctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    gctx.fillStyle = C.grid;
    for (let y = 0; y < H; y += P) for (let x = 0; x < W; x += P) gctx.fillRect(x + OFF, y + OFF, D, D);
  }

  const dot = (cx, cy, color, alpha) => {
    ctx.globalAlpha = alpha; ctx.fillStyle = color;
    ctx.fillRect(cx * P + OFF, cy * P + OFF, D, D);
  };
  const bigDot = (cx, cy, color, alpha) => {
    const S = P - 2;
    ctx.globalAlpha = alpha; ctx.fillStyle = color;
    ctx.fillRect(cx * P + 1, cy * P + 1, S, S);
  };
  const text = (s, x, y, color, font = FONT, align = 'left', alpha = 1) => {
    ctx.globalAlpha = alpha; ctx.font = font; ctx.fillStyle = color; ctx.textAlign = align;
    ctx.fillText(s, x, y);
  };
  const fmtSize = (n) => (n >= 1000 ? (n / 1000).toFixed(1) + 'K' : String(Math.round(n)));

  function draw() {
    if (!W || !H) return;
    ctx.globalAlpha = 1;
    ctx.clearRect(0, 0, W, H);
    ctx.drawImage(grid, 0, 0, W, H);
    ctx.textBaseline = 'middle';

    const cols = Math.floor(W / P), rows = Math.floor(H / P);
    const showPanel = W >= o.panelMinWidth;
    const panelCols = showPanel ? Math.min(34, Math.floor(cols * 0.22)) : 0;
    const panelC0 = cols - panelCols - 2;
    const headC = showPanel ? panelC0 - 5 : cols - 3;
    const r0 = Math.floor(rows * o.chartTop), r1 = rows - 3;   // chart lives in the lower half, under the text

    // scale
    const n = Math.min(hist.length, headC + 1);
    let lo = Math.min(50, v), hi = Math.max(50, v);
    for (let j = 1; j <= n; j++) { const x = hist[hist.length - j]; if (x < lo) lo = x; if (x > hi) hi = x; }
    if (hi - lo < o.minRange) { const c = (hi + lo) / 2; lo = c - o.minRange / 2; hi = c + o.minRange / 2; }
    if (sMin === null) { sMin = lo; sMax = hi; }
    sMin += (lo - sMin) * 0.04; sMax += (hi - sMax) * 0.04;
    const rowOf = (p) => clamp(r0 + Math.round((sMax - p) / (sMax - sMin) * (r1 - r0)), r0, r1);
    const r50 = rowOf(50);

    // 50% reference
    for (let c = (headC % 2); c <= headC; c += 2) dot(c, r50, C.accent, 0.35);

    // stepped chart with dotted fill
    let prev = null;
    for (let c = headC - n; c <= headC; c++) {
      if (c < 0) { continue; }
      const j = headC - c;
      const p = j === 0 ? v : hist[hist.length - j];
      if (p === undefined) continue;
      const r = rowOf(p);
      const col = p >= 50 ? C.up : C.down;
      for (let rr = r + 1; rr <= r1; rr++) {
        if ((c + rr) % 2) continue;
        dot(c, rr, col, 0.16 * (1 - (rr - r) / (r1 - r + 1)));
      }
      if (prev !== null && prev !== r) {
        const a = Math.min(prev, r), b = Math.max(prev, r);
        for (let rr = a; rr <= b; rr++) dot(c, rr, col, 0.85);
      }
      dot(c, r, col, 0.95);
      prev = r;
    }
    // head pulse
    const hr = rowOf(v), pulse = (Math.sin(t / 260) + 1) / 2;
    const hc = v >= 50 ? C.up : C.down;
    dot(headC, hr, C.bright, 1);
    for (const [dx, dy] of [[1, 0], [-1, 0], [0, 1], [0, -1]]) dot(headC + dx, hr + dy, hc, 0.25 + pulse * 0.5);
    for (const [dx, dy] of [[2, 0], [0, 2], [0, -2], [1, 1], [1, -1]]) dot(headC + dx, hr + dy, hc, pulse * 0.25);

    if (showPanel) {
      const x0 = panelC0 * P, x1 = (panelC0 + panelCols) * P;
      const pct = Math.round(v);
      const col = v >= 50 ? C.up : C.down;

      // big chance number
      const s = pct + '%';
      const R = 3;
      [...s].forEach((ch, i) => {
        const g = GLYPHS[ch];
        for (let gy = 0; gy < 7; gy++) for (let gx = 0; gx < 5; gx++) {
          if (g[gy][gx] === '1') bigDot(panelC0 + i * 6 + gx, R + gy, col, 0.95);
        }
      });
      text('chance', x0 + s.length * 6 * P + 4, (R + 6) * P + P / 2, C.text, FONT, 'left');

      // market + yes/no prices
      text(o.market, x0, (R + 9) * P + P / 2, C.text);
      text(`Yes ${pct}¢`, x0, (R + 11) * P + P, C.up, FONT_B);
      text(`No ${100 - pct + 1}¢`, x0 + 76, (R + 11) * P + P, C.down, FONT_B);

      // order book ladder
      const bookR = R + 15;
      const nLv = clamp(Math.floor((rows - bookR - 5) / 6), 2, 6);
      const mid = Math.round(v);
      const barC = panelC0 + 5;
      const barMax = panelCols - 5 - 6;
      const asks = [], bids = [];
      for (let i = nLv; i >= 1; i--) asks.push([mid + i, level(mid + i)]);
      for (let i = 0; i < nLv; i++) bids.push([mid - i, level(mid - i)]);
      let maxSize = 1;
      for (const [, L] of [...asks, ...bids]) maxSize = Math.max(maxSize, L.size);

      const drawRow = (price, L, row, color) => {
        const len = Math.max(1, Math.round((L.size / maxSize) * barMax));
        const yMid = (row + 1) * P;
        text(price + '¢', x0, yMid, C.text);
        for (let i = 0; i < len; i++) for (let k = 0; k < 2; k++) {
          dot(barC + i, row + k, L.flash > 0 ? C.bright : color, L.flash > 0 ? 0.35 + L.flash * 0.6 : 0.75);
        }
        text(fmtSize(L.size), x1, yMid, C.text, FONT, 'right', 0.8);
      };
      let row = bookR;
      text('Price', x0, row * P - P, C.text, FONT, 'left', 0.6);
      text('Shares', x1, row * P - P, C.text, FONT, 'right', 0.6);
      row += 1;
      for (const [p, L] of asks) { drawRow(p, L, row, C.down); row += 3; }
      text('spread 1¢', x0 + (panelCols * P) / 2, row * P + P / 2, C.text, FONT, 'center', 0.7);
      for (let c = panelC0; c < panelC0 + panelCols; c += 2) {
        if (Math.abs(c - (panelC0 + panelCols / 2)) > 6) dot(c, row, C.text, 0.3);
      }
      row += 2;
      for (const [p, L] of bids) { drawRow(p, L, row, C.up); row += 3; }
    }
    ctx.globalAlpha = 1;
    ctx.textAlign = 'left';
  }

  function resize() {
    const r = container.getBoundingClientRect();
    dpr = Math.min(window.devicePixelRatio || 1, 2);
    W = r.width; H = r.height;
    cvs.width = Math.round(W * dpr); cvs.height = Math.round(H * dpr);
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    buildGrid();
    draw();
  }
  const ro = new ResizeObserver(resize);
  ro.observe(container);
  resize();
  if (document.fonts && document.fonts.ready) document.fonts.ready.then(draw);
  const io = new IntersectionObserver(([e]) => { visible = e.isIntersecting; });
  io.observe(container);

  function frame(now) {
    const dt = Math.min(100, now - (last || now));
    last = now;
    if (running && visible) {
      const d = dt * speed;
      t += d;
      tickAcc += d; stepAcc += d;
      while (tickAcc >= o.tickMs) { tickAcc -= o.tickMs; tickPrice(); tickBook(); }
      while (stepAcc >= o.step) {
        stepAcc -= o.step;
        hist.push(v);
        if (hist.length > 800) hist.splice(0, 300);
      }
      draw();
    }
    raf = requestAnimationFrame(frame);
  }
  raf = requestAnimationFrame(frame);

  return {
    pause() { running = false; },
    play() { running = true; },
    destroy() { cancelAnimationFrame(raf); ro.disconnect(); io.disconnect(); cvs.remove(); },
  };
}
