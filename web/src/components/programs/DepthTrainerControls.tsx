import { useState } from 'react'
import { useSendCommand } from '../../hooks/useSendCommand'
import { useStatus } from '../../hooks/useDeviceState'
import { fmtClock, fmtLongCountdown, phaseLabel } from '../../lib/depthTrainer'
import type { DepthHistoryEntry, DepthProgress, DepthRating, DepthTrainerState } from '../../types/sscp'
import { DepthTrainerManual } from './DepthTrainerManual'

const RATING_LABELS: Record<DepthRating, string> = {
  comfortable: 'Comfortable',
  okay: 'Okay',
  too_much: 'Too much',
}

/**
 * Depth Trainer controls.
 *
 * Safety: this UI deliberately has NO control that moves the rod and no
 * pointer-hold / heartbeat button. Motion only happens while the hardware hand
 * switch is held. The UI can arm (start), emergency-retract (stop), edit the
 * breath-hold / safe-offset settings, rate a session and confirm a step back.
 */
export function DepthTrainerControls() {
  const { mode, depthProgress, depthTrainer } = useStatus()
  const isActive = mode === 'depth_trainer'

  return (
    <div className="flex flex-col gap-6 p-4">
      {isActive
        ? <ActiveView live={depthTrainer} progress={depthProgress} />
        : <IdleView progress={depthProgress} />}
      <Manual />
    </div>
  )
}

// ── Idle ──────────────────────────────────────────────────────────────────────

function IdleView({ progress }: { progress: DepthProgress | undefined }) {
  const send = useSendCommand()

  // Local edit state, synced from the device when the persisted value changes
  // (derive-during-render pattern, as the other program panels do).
  // breathHoldS is null until set; breathOffsetMm 0 is a valid (most
  // conservative) value, so the offset field is always populated.
  const devBreath = progress?.breathHoldS ?? null
  const devOffset = progress?.breathOffsetMm ?? 0
  const [breath, setBreath] = useState(devBreath !== null ? String(devBreath) : '')
  const [offset, setOffset] = useState(progress ? String(devOffset) : '')
  const [prevDev, setPrevDev] = useState({ devBreath, devOffset })
  if (prevDev.devBreath !== devBreath || prevDev.devOffset !== devOffset) {
    setPrevDev({ devBreath, devOffset })
    setBreath(devBreath !== null ? String(devBreath) : '')
    setOffset(String(devOffset))
  }

  const breathNum = parseFloat(breath)
  const offsetNum = parseFloat(offset)
  const inputsValid = Number.isFinite(breathNum) && breathNum > 0 && Number.isFinite(offsetNum) && offsetNum >= 0
  const dirty = breathNum !== devBreath || offsetNum !== devOffset

  const canStart = !!progress?.canStart && !dirty
  const blockReason = dirty
    ? 'Save the breath-hold and offset settings first.'
    : progress?.blockReason ?? (progress ? undefined : 'Waiting for the device to report Depth Trainer state.')

  function saveSettings() {
    if (!inputsValid) return
    send({ type: 'depth_settings', breathHoldS: breathNum, breathOffsetMm: offsetNum })
  }

  function start() {
    if (!canStart) return
    send({ type: 'depth_start' })
  }

  return (
    <>
      <div className="flex items-center gap-2">
        <span className="inline-block w-2 h-2 rounded-full bg-slate-600" />
        <span className="text-xs text-slate-400">Not running — start arms a session; the hand switch moves the rod</span>
      </div>

      {progress && <ProgressSummary progress={progress} />}

      {progress?.suggestStepBack && (
        <div className="flex flex-col gap-2 rounded-2xl bg-amber-950/40 border border-amber-700/50 p-3">
          <p className="text-xs text-amber-200 leading-snug">
            Two sessions in a row ended with an early release. Consider stepping back one
            progression step. It only happens if you confirm.
          </p>
          <button
            onClick={() => send({ type: 'depth_step_back' })}
            className="py-2 rounded-xl text-xs font-semibold bg-amber-600/80 hover:bg-amber-500 text-white transition-colors"
          >
            Confirm step back
          </button>
        </div>
      )}

      {/* Required settings */}
      <div className="flex flex-col gap-3 rounded-2xl bg-slate-800/60 border border-slate-700/60 p-4">
        <span className="text-[10px] uppercase tracking-widest text-slate-500">Settings (required)</span>
        <NumberField
          label="Breath-hold"
          unit="s"
          value={breath}
          onChange={setBreath}
          step={1}
          hint="Your comfortable breath-hold; the hold cap is min(0.6 x this, 20 s)"
        />
        <NumberField
          label="Breathing-safe offset"
          unit="mm"
          value={offset}
          onChange={setOffset}
          step={1}
          hint="mm beyond the origin from which you can breathe; 0 = origin (most conservative). Strokes turn around here."
        />
        <button
          onClick={saveSettings}
          disabled={!inputsValid || !dirty}
          className="py-2 rounded-xl text-xs font-semibold border border-slate-600 text-slate-200 hover:bg-slate-700 disabled:opacity-40 disabled:hover:bg-transparent transition-colors"
        >
          Save settings
        </button>
      </div>

      {/* Start */}
      <div className="flex flex-col gap-2">
        {!canStart && blockReason && (
          <p className="rounded-xl bg-slate-800/60 border border-slate-700/60 px-3 py-2 text-xs text-amber-300 leading-snug">
            {blockReason}
          </p>
        )}
        <button
          onClick={start}
          disabled={!canStart}
          className="flex items-center justify-center gap-2 w-full py-3 rounded-2xl font-semibold text-sm transition-all bg-purple-600 hover:bg-purple-500 text-white shadow-lg shadow-purple-500/20 disabled:opacity-40 disabled:shadow-none disabled:hover:bg-purple-600"
        >
          Start (arm) session
        </button>
        <p className="text-[11px] text-slate-500 text-center leading-snug">
          Starting only arms the trainer. Nothing moves until you hold the hand switch.
        </p>
      </div>

      {progress && progress.history.length > 0 && <History entries={progress.history} />}
    </>
  )
}

function ProgressSummary({ progress }: { progress: DepthProgress }) {
  return (
    <div className="flex flex-col gap-2 rounded-2xl bg-slate-800/60 border border-slate-700/60 p-4">
      <span className="text-[10px] uppercase tracking-widest text-slate-500">Progress</span>
      <Row label="Oscillation depth" value={`${progress.oscillationDepthMm} mm`} />
      <Row label="Hold depth" value={`${progress.holdDepthMm} mm`} />
      <Row
        label="Hold time / cap"
        value={progress.holdCapS !== null ? `${progress.holdS} s / ${progress.holdCapS.toFixed(1)} s` : `${progress.holdS} s / —`}
      />
      <Row label="Sessions" value={String(progress.sessions)} />
      <Row label="Next advance" value={fmtLongCountdown(progress.nextAdvanceInS)} />
    </div>
  )
}

function History({ entries }: { entries: DepthHistoryEntry[] }) {
  return (
    <div className="flex flex-col gap-2">
      <span className="text-[10px] uppercase tracking-widest text-slate-500">Recent sessions</span>
      <ul className="flex flex-col divide-y divide-slate-800 rounded-xl bg-slate-800/40 border border-slate-700/50">
        {entries.slice(-5).reverse().map((e, i) => (
          <li key={`${e.atMs}-${i}`} className="flex items-center justify-between gap-2 px-3 py-2 text-[11px]">
            <span className="text-slate-400">{new Date(e.atMs).toLocaleDateString()}</span>
            <span className="font-mono text-slate-300">
              {e.oscillationDepthMm}/{e.holdDepthMm} mm · {e.holdS} s
            </span>
            <span className={e.earlyReleases > 0 ? 'text-amber-400' : 'text-slate-500'}>
              {e.earlyReleases > 0 ? `${e.earlyReleases} early` : 'clean'}
              {e.rating ? ` · ${RATING_LABELS[e.rating as DepthRating] ?? e.rating}` : ''}
            </span>
          </li>
        ))}
      </ul>
    </div>
  )
}

// ── Active ────────────────────────────────────────────────────────────────────

function ActiveView({ live, progress }: { live: DepthTrainerState | undefined; progress: DepthProgress | undefined }) {
  const send = useSendCommand()

  const stopButton = (
    <div className="sticky bottom-0 -mx-4 px-4 py-3 bg-slate-900/95 border-t border-red-900/60 backdrop-blur z-10">
      <button
        onClick={() => send({ type: 'depth_stop' })}
        className="flex items-center justify-center gap-2 w-full py-5 rounded-2xl font-bold text-lg bg-red-600 hover:bg-red-500 active:bg-red-700 text-white shadow-lg shadow-red-900/40 transition-colors"
      >
        <svg viewBox="0 0 24 24" className="w-6 h-6" fill="currentColor" stroke="none">
          <rect x="6" y="6" width="12" height="12" rx="1" />
        </svg>
        Retract now / Stop
      </button>
    </div>
  )

  if (!live) {
    return (
      <>
        <p className="text-sm text-slate-300">Armed — waiting for the device to report session state…</p>
        {stopButton}
      </>
    )
  }

  const { phase } = live
  const resting = phase === 'rest' || live.restRemainingS > 0
  const capS = live.submergedCapS
  const capFrac = capS !== null && capS > 0 ? live.submergedS / capS : 0
  const submergedColor = capFrac >= 0.9 ? 'text-red-400' : capFrac >= 0.7 ? 'text-amber-300' : 'text-purple-300'

  return (
    <>
      {live.emergency && (
        <div role="alert" className="rounded-2xl bg-red-600/90 border border-red-300/60 px-4 py-3 text-white">
          <div className="text-sm font-bold uppercase tracking-wide">Emergency retract</div>
          <div className="text-xs mt-0.5">{live.emergencyReason ?? 'The trainer is retracting the rod to the origin.'}</div>
        </div>
      )}

      {live.awaitingDeadman && !live.emergency && (
        <div role="status" className="rounded-2xl bg-amber-500/20 border border-amber-400/60 px-4 py-3 text-amber-200 animate-pulse">
          <div className="text-sm font-semibold">Hold the hand switch to continue</div>
          <div className="text-xs mt-0.5">Releasing it retracts the rod.</div>
        </div>
      )}

      {live.awaitingWarmupConfirm && !live.emergency && (
        <div className="flex flex-col gap-3 rounded-2xl bg-purple-950/50 border border-purple-400/50 p-4">
          <div className="text-sm font-semibold text-purple-100">Warm-up done. Ready for the main session?</div>
          <p className="text-xs text-purple-200/80 leading-snug">
            The rod is parked at the origin and will not move until you choose. After
            Continue or +1 min more you must hold the hand switch again; a 3 s countdown
            runs first.
          </p>
          <div className="grid grid-cols-2 gap-2">
            <button
              onClick={() => send({ type: 'depth_warmup_continue' })}
              className="py-3 rounded-xl text-sm font-semibold bg-purple-600 hover:bg-purple-500 text-white transition-colors"
            >
              Continue
            </button>
            <button
              onClick={() => send({ type: 'depth_warmup_extend' })}
              disabled={!live.warmupCanExtend}
              className="py-3 rounded-xl text-sm font-semibold border border-purple-400/60 text-purple-100 hover:bg-purple-900/60 disabled:opacity-40 disabled:hover:bg-transparent transition-colors"
            >
              +1 min more
            </button>
          </div>
          {!live.warmupCanExtend && (
            <p className="text-[11px] text-slate-400">Maximum warm-up (10 min) reached.</p>
          )}
        </div>
      )}

      {phase === 'warmup' && live.warmupRemainingS !== null && live.warmupRemainingS > 0 && (
        <div className="flex flex-col items-center gap-1 rounded-2xl bg-slate-800/60 border border-slate-700/60 p-4">
          <span className="text-[10px] uppercase tracking-widest text-slate-500 text-center">
            Warming up — until the work-piece is lubricated
          </span>
          <span className="text-4xl font-mono font-bold tabular-nums text-purple-300">
            {fmtClock(live.warmupRemainingS)}
          </span>
        </div>
      )}

      <div className="flex flex-col gap-1">
        <span className="text-[10px] uppercase tracking-widest text-slate-500">
          Phase · hand switch {live.deadmanHeld ? 'held' : 'not held'}
        </span>
        <span className="text-lg font-semibold text-slate-100">{phaseLabel(phase)}</span>
        <span className="text-xs text-slate-400 font-mono">
          {live.setCount > 0 && <>Set {live.setIndex}/{live.setCount}</>}
          {live.setCount > 0 && live.strokeCount > 0 && ' · '}
          {live.strokeCount > 0 && <>Stroke {live.strokeIndex}/{live.strokeCount}</>}
          {live.plannedHoldS !== null && <> · hold {live.plannedHoldS.toFixed(0)} s</>}
        </span>
      </div>

      {/* Submerged countdown */}
      <div className="flex flex-col items-center gap-1 rounded-2xl bg-slate-800/60 border border-slate-700/60 p-4">
        <span className="text-[10px] uppercase tracking-widest text-slate-500">Time submerged</span>
        <span className={`text-6xl font-mono font-bold tabular-nums ${submergedColor}`}>
          {live.submergedS.toFixed(1)}
          <span className="text-2xl text-slate-500"> s</span>
        </span>
        <span className="text-xs text-slate-400 font-mono">
          {capS !== null
            ? <>cap {capS.toFixed(1)} s · {Math.max(0, capS - live.submergedS).toFixed(1)} s left</>
            : 'cap not set'}
        </span>
        <div className="mt-1 h-2 w-full rounded-full bg-slate-700 overflow-hidden">
          <div
            className={`h-full ${capFrac >= 0.9 ? 'bg-red-500' : capFrac >= 0.7 ? 'bg-amber-400' : 'bg-purple-400'}`}
            style={{ width: `${Math.min(100, Math.max(0, capFrac * 100))}%` }}
          />
        </div>
      </div>

      {/* Rest countdown */}
      {resting && (
        <div className="flex flex-col items-center gap-1 rounded-2xl bg-slate-800/60 border border-slate-700/60 p-4">
          <span className="text-[10px] uppercase tracking-widest text-slate-500">Rest</span>
          <span className="text-4xl font-mono font-bold tabular-nums text-sky-300">{fmtClock(live.restRemainingS)}</span>
          <span className="text-xs text-slate-400 font-mono">of {fmtClock(live.restTotalS)}</span>
          <div className="mt-1 h-2 w-full rounded-full bg-slate-700 overflow-hidden">
            <div
              className="h-full bg-sky-400"
              style={{
                width: `${live.restTotalS > 0 ? Math.min(100, Math.max(0, (1 - live.restRemainingS / live.restTotalS) * 100)) : 0}%`,
              }}
            />
          </div>
        </div>
      )}

      <DepthGauge
        depthMm={live.depthBeyondOriginMm}
        targetMm={live.targetDepthMm}
        safeOffsetMm={progress?.breathOffsetMm}
      />

      {phase === 'rating' && (
        <div className="flex flex-col gap-2">
          <span className="text-sm text-slate-200 font-semibold">How was this session?</span>
          <div className="grid grid-cols-3 gap-2">
            {(['comfortable', 'okay', 'too_much'] as const).map((r) => (
              <button
                key={r}
                onClick={() => send({ type: 'depth_rate', rating: r })}
                className={`py-3 rounded-xl text-xs font-semibold text-white transition-colors ${
                  r === 'comfortable' ? 'bg-emerald-600 hover:bg-emerald-500'
                  : r === 'okay' ? 'bg-slate-600 hover:bg-slate-500'
                  : 'bg-amber-700 hover:bg-amber-600'
                }`}
              >
                {RATING_LABELS[r]}
              </button>
            ))}
          </div>
        </div>
      )}

      {live.earlyRelease && (
        <p className="text-[11px] text-amber-400">Early release recorded — this session will not advance progress.</p>
      )}

      {stopButton}
    </>
  )
}

function DepthGauge({ depthMm, targetMm, safeOffsetMm }: { depthMm: number; targetMm: number; safeOffsetMm?: number }) {
  const scaleMax = Math.max(targetMm, depthMm, safeOffsetMm ?? 0, 1) * 1.15
  const pct = (v: number) => `${Math.min(100, Math.max(0, (v / scaleMax) * 100))}%`

  return (
    <div className="flex flex-col gap-2 rounded-2xl bg-slate-800/60 border border-slate-700/60 p-4">
      <div className="flex items-baseline justify-between">
        <span className="text-[10px] uppercase tracking-widest text-slate-500">Depth beyond origin</span>
        <span className="text-sm font-mono text-slate-200">
          {depthMm.toFixed(1)} <span className="text-slate-500">/ {targetMm.toFixed(1)} mm</span>
        </span>
      </div>
      <div className="relative h-4 rounded-full bg-slate-700 overflow-hidden">
        <div className="absolute inset-y-0 left-0 bg-purple-500" style={{ width: pct(Math.max(0, depthMm)) }} />
        <div className="absolute inset-y-0 w-0.5 bg-white" style={{ left: pct(targetMm) }} title="Target" />
        {safeOffsetMm !== undefined && safeOffsetMm > 0 && (
          <div className="absolute inset-y-0 w-0.5 bg-sky-300" style={{ left: pct(safeOffsetMm) }} title="Breathing-safe offset" />
        )}
      </div>
      <div className="flex justify-between text-[10px] text-slate-500">
        <span>origin</span>
        <span>
          <span className="text-white">|</span> target
          {safeOffsetMm !== undefined && safeOffsetMm > 0 && <> · <span className="text-sky-300">|</span> safe offset</>}
        </span>
      </div>
    </div>
  )
}

// ── Shared bits ───────────────────────────────────────────────────────────────

function Row({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-center justify-between gap-2">
      <span className="text-xs text-slate-400">{label}</span>
      <span className="text-sm font-semibold text-purple-200 font-mono">{value}</span>
    </div>
  )
}

function NumberField({
  label, unit, value, onChange, step, hint,
}: {
  label: string
  unit: string
  value: string
  onChange: (v: string) => void
  step: number
  hint: string
}) {
  return (
    <label className="flex flex-col gap-1">
      <span className="flex items-center justify-between text-sm text-slate-300">
        {label}
        <span className="flex items-center gap-1.5">
          <input
            type="number"
            inputMode="decimal"
            min={0}
            step={step}
            value={value}
            onChange={(e) => onChange(e.target.value)}
            className="w-20 rounded-lg bg-slate-900 border border-slate-600 px-2 py-1 text-right font-mono text-sm text-slate-100 focus:outline-none focus:border-purple-400"
          />
          <span className="text-xs text-slate-500 w-6">{unit}</span>
        </span>
      </span>
      <span className="text-[11px] text-slate-500 leading-snug">{hint}</span>
    </label>
  )
}

function Manual() {
  const [show, setShow] = useState(false)
  return (
    <div className="flex flex-col gap-3 border-t border-slate-800 pt-4">
      <button
        onClick={() => setShow((v) => !v)}
        className="flex items-center justify-center gap-1.5 text-xs font-medium text-slate-400 hover:text-slate-200 transition-colors"
      >
        <svg
          viewBox="0 0 24 24"
          className={`w-3.5 h-3.5 transition-transform ${show ? 'rotate-180' : ''}`}
          fill="none" stroke="currentColor" strokeWidth={2}
        >
          <polyline points="6 9 12 15 18 9" strokeLinecap="round" strokeLinejoin="round" />
        </svg>
        {show ? 'Hide how it works' : 'How it works'}
      </button>
      {show && <DepthTrainerManual />}
    </div>
  )
}
