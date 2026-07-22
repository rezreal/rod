import { create } from 'zustand'

export interface ErrorToast {
  id: number
  message: string
}

interface ErrorStore {
  toasts: ErrorToast[]
  pushError(message: string): void
  dismissError(id: number): void
}

let nextId = 0

export const useErrorStore = create<ErrorStore>((set) => ({
  toasts: [],
  pushError: (message) =>
    set((s) => ({ toasts: [...s.toasts, { id: nextId++, message }] })),
  dismissError: (id) =>
    set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) })),
}))
