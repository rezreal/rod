import { useErrorStore } from '../store/errorStore'

function formatError(reason: unknown): string {
  if (reason instanceof Error) return reason.message
  if (typeof reason === 'string') return reason
  try {
    return JSON.stringify(reason)
  } catch {
    return String(reason)
  }
}

/** Surfaces uncaught exceptions and unhandled promise rejections as UI toasts. */
export function installGlobalErrorHandler() {
  window.addEventListener('error', (event) => {
    useErrorStore.getState().pushError(formatError(event.error ?? event.message))
  })

  window.addEventListener('unhandledrejection', (event) => {
    useErrorStore.getState().pushError(formatError(event.reason))
  })
}
