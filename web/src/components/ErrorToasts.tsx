import { useEffect } from 'react'
import { useErrorStore } from '../store/errorStore'

const AUTO_DISMISS_MS = 8000

export function ErrorToasts() {
  const toasts = useErrorStore((s) => s.toasts)
  const dismissError = useErrorStore((s) => s.dismissError)

  return (
    <div className="fixed bottom-4 right-4 z-50 flex flex-col gap-2 max-w-sm">
      {toasts.map((toast) => (
        <ErrorToastItem key={toast.id} id={toast.id} message={toast.message} onDismiss={dismissError} />
      ))}
    </div>
  )
}

function ErrorToastItem({ id, message, onDismiss }: { id: number; message: string; onDismiss: (id: number) => void }) {
  useEffect(() => {
    const timer = setTimeout(() => onDismiss(id), AUTO_DISMISS_MS)
    return () => clearTimeout(timer)
  }, [id, onDismiss])

  return (
    <div className="flex items-start gap-3 px-4 py-3 bg-red-900/90 border border-red-700 text-red-200 text-sm rounded-xl shadow-lg backdrop-blur">
      <svg viewBox="0 0 24 24" className="w-5 h-5 shrink-0 mt-0.5" fill="none" stroke="currentColor" strokeWidth={2}>
        <path strokeLinecap="round" strokeLinejoin="round" d="M12 9v3.75m9.303 3.376c.866 1.5-.217 3.374-1.948 3.374H4.645c-1.73 0-2.813-1.874-1.948-3.374l7.3-12.748a2.25 2.25 0 0 1 3.898 0l7.3 12.748z" />
      </svg>
      <span className="flex-1 break-words">{message}</span>
      <button onClick={() => onDismiss(id)} className="text-red-400 hover:text-red-200 shrink-0">
        <svg viewBox="0 0 24 24" className="w-4 h-4" fill="none" stroke="currentColor" strokeWidth={2}>
          <path strokeLinecap="round" strokeLinejoin="round" d="M6 18 18 6M6 6l12 12" />
        </svg>
      </button>
    </div>
  )
}
