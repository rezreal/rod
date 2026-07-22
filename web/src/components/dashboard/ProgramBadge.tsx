import type { ProgramMode } from '../../types/sscp'
import { useStatus } from '../../hooks/useDeviceState'

const MODE_LABELS: Record<ProgramMode, string> = {
  idle:   'IDLE',
  hamp:   'HAMP',
  hdsp:   'TOUCHPAD',
  hsp:    'SCRIPT',
  homing: 'HOMING',
  drill:  'DRILL',
  ramp:   'RAMP',
  game:   'GAME',
  cycle:  'CYCLE',
  learn:  'LEARN',
  pulse:  'PULSE',
  impale: 'IMPALE',
  plumb:  'PLUMB',
  surge:  'SURGE',
  tide:   'TIDE',
  echo:   'ECHO',
  trace:  'TRACE',
  tempo:  'TEMPO',
}

const MODE_COLORS: Record<ProgramMode, string> = {
  idle:   'bg-slate-700 text-slate-400',
  hamp:   'bg-cyan-500/20 text-cyan-400 border border-cyan-500/30',
  hdsp:   'bg-violet-500/20 text-violet-400 border border-violet-500/30',
  hsp:    'bg-emerald-500/20 text-emerald-400 border border-emerald-500/30',
  homing: 'bg-amber-500/20 text-amber-400 border border-amber-500/30 animate-pulse',
  drill:  'bg-orange-500/20 text-orange-400 border border-orange-500/30',
  ramp:   'bg-rose-500/20 text-rose-400 border border-rose-500/30',
  game:   'bg-fuchsia-500/20 text-fuchsia-400 border border-fuchsia-500/30',
  cycle:  'bg-teal-500/20 text-teal-400 border border-teal-500/30',
  learn:  'bg-lime-500/20 text-lime-400 border border-lime-500/30',
  pulse:  'bg-red-500/20 text-red-400 border border-red-500/30',
  impale: 'bg-indigo-500/20 text-indigo-400 border border-indigo-500/30',
  plumb:  'bg-sky-500/20 text-sky-400 border border-sky-500/30',
  surge:  'bg-rose-500/20 text-rose-400 border border-rose-500/30',
  tide:   'bg-blue-500/20 text-blue-400 border border-blue-500/30',
  echo:   'bg-emerald-500/20 text-emerald-400 border border-emerald-500/30',
  trace:  'bg-violet-500/20 text-violet-400 border border-violet-500/30',
  tempo:  'bg-amber-500/20 text-amber-400 border border-amber-500/30',
}

export function ProgramBadge() {
  const { mode } = useStatus()

  return (
    <span className={`inline-flex items-center px-2.5 py-1 rounded-full text-xs font-semibold tracking-wider ${MODE_COLORS[mode]}`}>
      {MODE_LABELS[mode]}
    </span>
  )
}
