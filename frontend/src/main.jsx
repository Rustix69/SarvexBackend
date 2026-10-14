import { StrictMode, lazy, Suspense } from 'react'
import { createRoot } from 'react-dom/client'
import './index.css'

const App = lazy(() => import('./App.jsx'))

// "/" is the marketing landing page (a standalone page in /public); the app lives on /discover and the other routes.
function Landing() {
  return <iframe title="SarvaEX" src="/landing.html" style={{ position: 'fixed', inset: 0, width: '100%', height: '100%', border: 0, background: '#0a0a0a' }} />
}

createRoot(document.getElementById('root')).render(
  <StrictMode>
    {window.location.pathname === '/' ? <Landing /> : <Suspense fallback={null}><App /></Suspense>}
  </StrictMode>,
)
