import { useDeviceState } from '../../hooks/useDeviceState'
import { CycleControls } from './CycleControls'
import { DrillControls } from './DrillControls'
import { GamesControls } from './GamesControls'
import { HampControls } from './HampControls'
import { TouchpadControls } from './TouchpadControls'
import { HspControls } from './HspControls'
import { ImpaleControls } from './ImpaleControls'
import { LearnControls } from './LearnControls'
import { PulseControls } from './PulseControls'
import { RampControls } from './RampControls'
import { PlumbControls } from './PlumbControls'
import { SurgeControls } from './SurgeControls'
import { TideControls } from './TideControls'
import { EchoControls } from './EchoControls'
import { TraceControls } from './TraceControls'
import { TempoControls } from './TempoControls'

const TITLES = {
  hamp:  'Oscillation',
  hdsp:  'Touchpad',
  hsp:   'Script',
  drill: 'Drill',
  ramp:  'Auto-Ramp',
  game:  'Endurance Games',
  cycle: 'Cycle',
  learn: 'Learn (teach & repeat)',
  pulse: 'Pulse (heart rate)',
  impale: 'Impale',
  plumb: 'Plumb',
  surge: 'Surge',
  tide: 'Tide',
  echo: 'Echo',
  trace: 'Trace',
  tempo: 'Tempo',
}

export function ProgramDrawer() {
  const { activeProgram } = useDeviceState()

  return (
    <div className="flex flex-col bg-slate-900 border-l border-slate-800 md:w-80 lg:w-96 shrink-0 overflow-y-auto">
      <div className="px-4 pt-4 pb-2 border-b border-slate-800">
        <h2 className="text-sm font-semibold text-slate-300">{TITLES[activeProgram]}</h2>
      </div>

      {activeProgram === 'hamp'  && <HampControls />}
      {activeProgram === 'hdsp'  && <TouchpadControls />}
      {activeProgram === 'hsp'   && <HspControls />}
      {activeProgram === 'drill' && <DrillControls />}
      {activeProgram === 'ramp'  && <RampControls />}
      {activeProgram === 'game'  && <GamesControls />}
      {activeProgram === 'cycle' && <CycleControls />}
      {activeProgram === 'learn' && <LearnControls />}
      {activeProgram === 'pulse' && <PulseControls />}
      {activeProgram === 'impale' && <ImpaleControls />}
      {activeProgram === 'plumb' && <PlumbControls />}
      {activeProgram === 'surge' && <SurgeControls />}
      {activeProgram === 'tide' && <TideControls />}
      {activeProgram === 'echo' && <EchoControls />}
      {activeProgram === 'trace' && <TraceControls />}
      {activeProgram === 'tempo' && <TempoControls />}
    </div>
  )
}
