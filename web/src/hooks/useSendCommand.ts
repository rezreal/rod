import { useTransport } from '../transport/TransportProvider'
import type { Command } from '../types/sscp'

export function useSendCommand() {
  const { send } = useTransport()
  return (cmd: Command) => send(cmd)
}
