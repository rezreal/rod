import { Component, type ErrorInfo, type ReactNode } from 'react'
import { useErrorStore } from '../store/errorStore'

interface Props {
  children: ReactNode
}

interface State {
  hasError: boolean
}

export class ErrorBoundary extends Component<Props, State> {
  state: State = { hasError: false }

  static getDerivedStateFromError(): State {
    return { hasError: true }
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    useErrorStore.getState().pushError(error.message)
    console.error(error, info)
  }

  render() {
    if (this.state.hasError) {
      return (
        <div className="h-dvh flex flex-col items-center justify-center gap-4 bg-[#0a0f1a] text-slate-100 p-6 text-center">
          <p className="text-lg font-semibold text-red-300">Something went wrong</p>
          <p className="text-sm text-slate-500 max-w-sm">The app hit an unexpected error and needs to reload.</p>
          <button
            onClick={() => window.location.reload()}
            className="px-4 py-2 bg-red-600 hover:bg-red-500 text-white text-sm font-semibold rounded-lg transition-colors"
          >
            Reload
          </button>
        </div>
      )
    }
    return this.props.children
  }
}
