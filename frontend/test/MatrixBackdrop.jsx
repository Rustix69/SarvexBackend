'use client';
// Design C. Drop inside your hero container (position: relative; overflow: hidden).
import { useEffect, useRef } from 'react';
import { mountMatrixBackdrop } from './matrix-backdrop';

const CSS = `
.mx-root{position:absolute;inset:0;overflow:hidden;pointer-events:none}
.mx-canvas{position:absolute;inset:0}
.mx-vignette{position:absolute;inset:0;background:
  radial-gradient(ellipse 46% 52% at var(--mx-focus,34% 55%), rgba(12,13,17,.95) 0%, rgba(12,13,17,.8) 45%, rgba(12,13,17,.15) 80%, rgba(12,13,17,0) 100%),
  linear-gradient(90deg, rgba(12,13,17,.85) 0%, rgba(12,13,17,0) 22%)}
@media (max-width:700px){.mx-vignette{background:linear-gradient(0deg,rgba(12,13,17,.92),rgba(12,13,17,.6) 70%,rgba(12,13,17,.35))}}
`;

export default function MatrixBackdrop({
  focus = '34% 55%',   // where your headline sits
  options = {},        // market name, colors, speed… see matrix-backdrop.js
}) {
  const host = useRef(null);
  useEffect(() => {
    const bd = mountMatrixBackdrop(host.current, options);
    return () => bd.destroy();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  return (
    <div className="mx-root" aria-hidden="true" style={{ '--mx-focus': focus }}>
      <style>{CSS}</style>
      <div className="mx-canvas" ref={host} />
      <div className="mx-vignette" />
    </div>
  );
}
