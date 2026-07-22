import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { TransportProvider } from './transport/TransportProvider'
import { App } from './App'
import { ErrorToasts } from './components/ErrorToasts'
import { ErrorBoundary } from './components/ErrorBoundary'
import { installGlobalErrorHandler } from './lib/globalErrorHandler'
import './index.css' // eslint-disable-line

installGlobalErrorHandler()

const root = document.getElementById('root')!
createRoot(root).render(
  <StrictMode>
    <TransportProvider>
      <ErrorBoundary>
        <App />
      </ErrorBoundary>
      <ErrorToasts />
    </TransportProvider>
  </StrictMode>,
)
