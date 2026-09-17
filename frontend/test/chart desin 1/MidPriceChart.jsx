'use client';
import { useEffect, useRef } from 'react';
import { mountMidPriceChart } from './mid-price-chart';

// getData(range) must return { points, fills, events, last, closeLabel } (see mid-price-chart.js).
// onReady gets the chart instance, so you can call chart.tick({ bid, ask, fill }) from your websocket.
export default function MidPriceChart({ getData, initialRange = '1D', height = 300, onReady }) {
  const ref = useRef(null);
  useEffect(() => {
    const chart = mountMidPriceChart(ref.current, { getData, initialRange, height });
    onReady?.(chart);
    return () => chart.destroy();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  return <div ref={ref} />;
}
