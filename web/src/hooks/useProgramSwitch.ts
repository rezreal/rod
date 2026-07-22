import { useDeviceState } from './useDeviceState'
import { useTransport } from '../transport/TransportProvider'
import type { ActiveProgram } from '../store/deviceStore'

/** Switching programs in the UI should immediately act like pressing STOP —
 *  a previously running mode must not keep moving in the background. */
export function useProgramSwitch() {
  const { activeProgram, setActiveProgram } = useDeviceState()
  const { send } = useTransport()

  function switchProgram(id: ActiveProgram) {
    if (id !== activeProgram) send({ type: 'stop_all' })
    setActiveProgram(id)
  }

  return { activeProgram, switchProgram }
}
