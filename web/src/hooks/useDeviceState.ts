import { useShallow } from 'zustand/react/shallow'
import { useDeviceStore } from '../store/deviceStore'

/** Position + waveform — re-renders at telemetry rate */
export function usePosition() {
  return useDeviceStore((s) => s.position)
}

/** Health bits, mode, HAMP params — only re-renders when something changes */
export function useStatus() {
  return useDeviceStore((s) => s.status)
}

/** Convenience hook for components that need both */
export function useDeviceState() {
  return useDeviceStore(
    useShallow((s) => ({
      connectionState:  s.connectionState,
      deviceInfo:       s.deviceInfo,
      activeProgram:    s.activeProgram,
      setActiveProgram: s.setActiveProgram,
      position:         s.position,
      status:           s.status,
    })),
  )
}
