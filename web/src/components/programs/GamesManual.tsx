import type { GameKind } from '../../types/sscp'

interface GameDoc {
  kind: GameKind
  name: string
  tagline: string
  rules: string[]
  operate: string[]
  meta: string
}

export const GAME_DOCS: GameDoc[] = [
  {
    kind: 'edge_recover',
    name: 'Edge & Recover',
    tagline: 'Stamina trainer',
    rules: [
      'Hold the button and the intensity (speed) climbs steadily.',
      'Release before you go over the edge to back off — intensity falls during recovery.',
      'Each release from a high intensity counts as an “edge”.',
      'Goal: rack up edges and total time.',
    ],
    operate: [
      'Start, then hold to climb / release to recover, repeatedly.',
      'Stop to end the round.',
    ],
    meta: 'Level = edges counted.',
  },
  {
    kind: 'hold_the_line',
    name: 'Hold the Line',
    tagline: 'Isometric resistance',
    rules: [
      'The motor presses the rod outward with a force that ramps up the longer you stay engaged; you resist by hand.',
      'Hold the button to engage.',
      'If the rod gets driven past your line you lose ground (a “line lost”) and it re-anchors forward.',
      'Release to yield and relax the push.',
      'Goal: hold the line as long as you can.',
    ],
    operate: [
      'Start, then hold to engage and resist the push.',
      'Release to relax. Stop to end the round.',
    ],
    meta: 'Level = lines lost (lower is better).',
  },
  {
    kind: 'gauntlet',
    name: 'The Gauntlet',
    tagline: 'Interval training',
    rules: [
      'Begins in a Rest phase with the rod free.',
      'When you’re ready, press and hold the button to start a Work interval (firm oscillation).',
      'Hold all the way through to complete the interval; releasing early aborts it with no credit.',
      'Each completed interval is longer than the last.',
      'If you never signal ready during a rest, the gauntlet ends.',
      'Goal: complete as many intervals as possible.',
    ],
    operate: [
      'During Rest, hold to begin the next Work interval.',
      'Keep holding until the interval completes. Stop to end.',
    ],
    meta: 'Level = intervals completed.',
  },
  {
    kind: 'deadmans_climb',
    name: "Deadman's Climb",
    tagline: 'Banked climb',
    rules: [
      'Hold the button to climb through intensity checkpoints; each checkpoint you pass is locked in (“banked”).',
      'If you let go or lapse, you only fall back to your last checkpoint instead of all the way to zero — then climb again.',
      'Bank the final checkpoint (100%) and you win — the round ends there.',
    ],
    operate: [
      'Start, then hold to climb. Release falls back to your last banked checkpoint.',
      'Hold again to keep climbing. Reach the top to win, or Stop to end early.',
    ],
    meta: 'Level = highest checkpoint reached; duration is how long the climb took.',
  },
  {
    kind: 'stillness',
    name: 'Stillness',
    tagline: 'Control challenge',
    rules: [
      'The servo is OFF so the rod moves freely in your hand — nothing tugs or drives it.',
      'Hold the button to stay in the round and keep the rod still — within a tolerance of where you started.',
      'Drift past tolerance and you get a quick micro-vibration warning and lose a life; the round then re-centers on your current spot.',
      'You start with 5 lives — lose them all and the round ends.',
      'Goal: stay still as long as possible.',
    ],
    operate: [
      'Start, then hold and hold the rod steady at its starting position.',
      'Feel a buzz? You drifted — settle back down. Stop to end.',
    ],
    meta: 'Level = lives remaining; duration is seconds survived.',
  },
]

export function GamesManual({ only }: { only?: GameKind }) {
  const docs = only ? GAME_DOCS.filter((d) => d.kind === only) : GAME_DOCS

  return (
    <div className="flex flex-col gap-5">
      {docs.map((doc) => (
        <div key={doc.kind} className="flex flex-col gap-2">
          <div className="flex items-baseline justify-between gap-2">
            <h3 className="text-sm font-semibold text-fuchsia-300">{doc.name}</h3>
            <span className="text-[10px] uppercase tracking-widest text-slate-500">{doc.tagline}</span>
          </div>

          <ul className="flex flex-col gap-1 text-xs text-slate-300 leading-relaxed">
            {doc.rules.map((line, i) => (
              <li key={i} className="flex gap-2">
                <span className="text-fuchsia-500/70 select-none">•</span>
                <span>{line}</span>
              </li>
            ))}
          </ul>

          <div className="flex flex-col gap-1">
            <span className="text-[10px] font-semibold uppercase tracking-widest text-slate-500">
              How to operate
            </span>
            <ol className="flex flex-col gap-1 text-xs text-slate-400 leading-relaxed">
              {doc.operate.map((line, i) => (
                <li key={i} className="flex gap-2">
                  <span className="text-slate-600 select-none font-mono">{i + 1}.</span>
                  <span>{line}</span>
                </li>
              ))}
            </ol>
          </div>

          <p className="text-[11px] text-slate-500 italic">{doc.meta}</p>
        </div>
      ))}

      {!only && (
        <div className="flex flex-col gap-1.5 rounded-xl bg-slate-800/60 border border-slate-700/60 p-3">
          <p className="text-xs text-slate-300">
            Starting a game arms it, but motion doesn't begin right away — tap the physical
            button on the device three times to confirm you're ready, then it starts after a
            short delay. This gesture only works on the device itself, not from the app.
          </p>
          <p className="text-xs text-slate-300">
            Once running, the button is a <span className="font-semibold text-fuchsia-300">deadman</span> — the
            game only runs while you hold it; let go and motion stops. Hit <span className="font-semibold">Stop</span> to
            leave the game.
          </p>
          <p className="text-[11px] text-amber-400/80">
            Safety: releasing the button always stops motion.
          </p>
        </div>
      )}
    </div>
  )
}
