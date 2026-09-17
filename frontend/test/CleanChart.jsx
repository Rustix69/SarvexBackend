'use client';
import { useEffect, useRef } from 'react';
import { mountCleanChart } from './clean-chart';

// getData(range) returns { points, fills, events, closeLabel } (same points format as design 1).
export default function CleanChart({ getData, initialRange = '1D', height = 260, onReady }) {
  const ref = useRef(null);
  useEffect(() => {
    const chart = mountCleanChart(ref.current, { getData, initialRange, height });
    onReady?.(chart);
    return () => chart.destroy();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  return <div ref={ref} />;
}
