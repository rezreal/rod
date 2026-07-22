interface PatternDoc {
  name: string
  desc: string
}

/** The 10 cycle patterns, in firmware order (index 0–9). */
export const CYCLE_PATTERNS: PatternDoc[] = [
  { name: 'Sine',          desc: 'Smooth, even full-range strokes.' },
  { name: 'Triangle',      desc: 'Constant-speed up-and-down strokes.' },
  { name: 'Sawtooth',      desc: 'Slow extend, quick retract.' },
  { name: 'Thrust & hold', desc: 'Push to the far end, dwell, return, dwell.' },
  { name: 'Tease',         desc: 'Small quick strokes hugging the near end.' },
  { name: 'Double-stroke', desc: 'Two short jabs, then one long stroke.' },
  { name: 'Build',         desc: 'Strokes that speed up across each cycle.' },
  { name: 'Crescendo',     desc: 'Pulses that grow from small to full.' },
  { name: 'Weave',         desc: 'Two detuned rhythms blended into a wandering motion.' },
  { name: 'Wander',        desc: 'Oscillates around the near end, travels out to oscillate around the far end, then returns and repeats.' },
  { name: 'Plunge',        desc: 'Fast, strong thrust to the far end, then a slow retract.' },
]

export function CycleManual({ current }: { current?: number }) {
  return (
    <div className="flex flex-col gap-5">
      {/* Operation */}
      <div className="flex flex-col gap-2">
        <span className="text-[10px] font-semibold uppercase tracking-widest text-slate-500">
          How it works
        </span>
        <ul className="flex flex-col gap-1 text-xs text-slate-300 leading-relaxed">
          <li className="flex gap-2">
            <span className="text-teal-500/70 select-none">•</span>
            <span>
              One button. <span className="font-semibold text-teal-300">Tap</span> (quick press) to advance
              to the next pattern.
            </span>
          </li>
          <li className="flex gap-2">
            <span className="text-teal-500/70 select-none">•</span>
            <span>
              <span className="font-semibold text-teal-300">Hold for 2 seconds</span> to pause or resume.
            </span>
          </li>
          <li className="flex gap-2">
            <span className="text-teal-500/70 select-none">•</span>
            <span>
              All patterns play over the same stroke zone (same origin and distance); they differ only in
              speed and shape.
            </span>
          </li>
          <li className="flex gap-2">
            <span className="text-teal-500/70 select-none">•</span>
            <span>
              Press <span className="font-semibold">Start</span> to begin, <span className="font-semibold">Stop</span> to leave.
            </span>
          </li>
        </ul>
      </div>

      {/* Patterns */}
      <div className="flex flex-col gap-2">
        <span className="text-[10px] font-semibold uppercase tracking-widest text-slate-500">
          The 10 patterns
        </span>
        <ol className="flex flex-col gap-1 text-xs leading-relaxed">
          {CYCLE_PATTERNS.map((p, i) => {
            const active = current === i
            return (
              <li
                key={i}
                className={`flex gap-2 rounded-lg px-2 py-1 transition-colors ${
                  active ? 'bg-teal-500/15 border border-teal-500/30' : 'border border-transparent'
                }`}
              >
                <span className={`select-none font-mono ${active ? 'text-teal-300' : 'text-slate-600'}`}>
                  {i + 1}.
                </span>
                <span>
                  <span className={`font-semibold ${active ? 'text-teal-200' : 'text-slate-200'}`}>
                    {p.name}
                  </span>
                  <span className="text-slate-400"> — {p.desc}</span>
                </span>
              </li>
            )
          })}
        </ol>
      </div>

      <p className="text-[11px] text-slate-500 italic">
        The bridge times the press, so a tap steps patterns and a 2-second hold toggles pause.
      </p>
    </div>
  )
}
