'use client';
import { useEffect, useRef } from 'react';
import { mountMidPriceChart } from './mid-price-chart';

// getData(range) must return { points, fills, events, last, closeLabel } (see mid-price-chart.js).
// onReady gets the chart instance, so you can call chart.tick({ bid, ask, fill }) from your websocket.
export default function MidPriceChart({ getData, initialRange = '1D', height = 300, onReady }) {
  const ref = useRef(null);
  const chartRef = useRef(null);
  const getDataRef = useRef(getData);

  useEffect(() => {
    const chart = mountMidPriceChart(ref.current, {
      getData: (range) => getDataRef.current(range),
      initialRange,
      height,
    });
    chartRef.current = chart;
    onReady?.(chart);
    return () => {
      chart.destroy();
      chartRef.current = null;
    };
  }, [height, initialRange, onReady]);

  useEffect(() => {
    getDataRef.current = getData;
    chartRef.current?.refresh();
  }, [getData]);

  return <div ref={ref} />;
}
