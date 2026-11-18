import { submergedCapS, mandatoryRestS } from '../../lib/depthTrainer'

/**
 * "How it works" text for Depth Trainer. Kept in its own file (like
 * CycleManual / GamesManual) so the controls stay readable. The numbers in the
 * worked example come from the same helpers the UI uses, so they can't drift
 * from the stated rules.
 */
export function DepthTrainerManual() {
  const exampleBreath = 30
  const exampleCap = submergedCapS(exampleBreath)
  const exampleRest = mandatoryRestS(10)

  return (
    <div className="flex flex-col gap-3 rounded-2xl bg-slate-800/40 border border-slate-700/50 p-4 text-xs text-slate-400 leading-relaxed">
      <p className="rounded-lg bg-amber-950/40 border border-amber-700/40 px-3 py-2 text-amber-300">
        <span className="font-semibold">Experimental.</span> Depth Trainer has not been
        tested on hardware. Treat every session as a trial run, stay in control of the
        hand switch, and stop at the first sign of discomfort.
      </p>

      <Section title="What it does">
        Over many sessions (weeks) it helps you get comfortable with a gradually
        increasing depth. Two exercises: <Em>oscillate</Em> at depth and <Em>hold</Em> at
        depth. Depths are measured in mm beyond the calibrated work origin (calibrate first).
      </Section>

      <Section title="The hand switch is the deadman">
        This page can <Em>arm</Em> a session, <Em>stop</Em> it and record settings and
        ratings, but it can never make the rod move. The rod only advances, oscillates
        or dwells while the <Em>hardware hand switch is held</Em>. Releasing it (or any
        gap in its signal) immediately retracts the rod to the origin and aborts the
        current set. Starting or resuming after a release needs the switch held again.
      </Section>

      <Section title="Breath-hold and breathing-safe offset (required)">
        Enter your comfortable breath-hold time in seconds and your <Em>breathing-safe
        offset</Em>: how many mm beyond the origin the rod can be while you can still
        breathe. You can't reliably breathe at the origin itself, so the offset defaults
        to 0 (the origin, the most conservative setting). Save both before starting; a
        session can't start until a breath-hold is set. They can't be changed while a
        session is running. You are "submerged" whenever the rod is beyond the origin
        plus this offset.
      </Section>

      <Section title="The time cap">
        Each excursion is capped at <Em>min(0.6 × breath-hold, 20 s)</Em>. The submerged
        clock includes the way out, the dwell and the way back. Example: a {exampleBreath} s
        breath-hold gives a {exampleCap} s cap. If a depth can't be reached within the
        cap the step is refused rather than silently shortened. If the timer reaches the
        cap the trainer retracts on its own. Oscillation strokes turn around at the
        breathing-safe offset, not the origin, and pause at least 2 s there (a breath
        window). The cumulative submerged time of a whole oscillation set is capped as
        well, so a set has fewer strokes at greater depth. Holds return to the origin.
        The return stroke runs at 10 mm/s; going deeper uses 4 mm/s, slowing to
        1.6 mm/s for the last 5 mm.
      </Section>

      <Section title="Mandatory rest">
        After every excursion you rest at the origin for{' '}
        <Em>max(3 × time submerged, 20 s)</Em>, e.g. {exampleRest} s after a 10 s excursion.
        The next move waits for the rest to finish <Em>and</Em> the hand switch to be held
        again.
      </Section>

      <Section title="Warm-up">
        Every session starts with a <Em>timed warm-up</Em> (default 2 min) so the
        work-piece can lubricate. It uses gentle strokes that stay inside the
        breathing-safe offset, so no breath limit or rest applies. When the time is up
        the rod parks and waits: <Em>you</Em> decide when you're ready, with{' '}
        <Em>Continue</Em>, or <Em>+1 min more</Em> up to 10 min in total. After either
        you must hold the hand switch again, and a 3 s countdown runs before anything
        moves. If the offset is under 2 mm there is no breath-safe room, so the short
        3-stroke warm-up runs instead (normal budget and rest rules) and there is no
        confirm step.
      </Section>

      <Section title="Session structure">
        Warm-up → oscillation sets → hold sets (3 holds, rest between) → cool-down (2
        shallow strokes, rod parked at the origin) → you rate the session:{' '}
        <Em>comfortable</Em>, <Em>okay</Em> or <Em>too much</Em>. A session takes roughly
        6–7 minutes with the default warm-up, longer as depth and hold time grow, plus
        any extra warm-up you add.
      </Section>

      <Section title="Progression">
        <ul className="list-disc pl-4 flex flex-col gap-1">
          <li>Start: oscillation 10 mm, hold 5 mm for 5 s.</li>
          <li>
            Rated <Em>comfortable</Em> with no early release, and at least 24 h since the
            last advance: hold time goes up <Em>1 s</Em> per session until it reaches the
            cap, then depth goes up <Em>2 mm</Em> and hold time resets to 5 s. Oscillation
            depth advances +2 mm on the same rule.
          </li>
          <li><Em>Okay</Em> keeps things as they are; <Em>too much</Em> steps back one step.</li>
          <li>Releasing the hand switch mid-set blocks progress for that session.</li>
          <li>
            After two sessions in a row with an early release the app suggests a step
            back; it only happens if you confirm.
          </li>
          <li>Nothing advances twice inside the 24 h cooldown, or beyond max depth.</li>
        </ul>
      </Section>

      <Section title="Limits you should know about">
        <p className="rounded-lg bg-rose-950/40 border border-rose-700/40 px-3 py-2 text-rose-300">
          If the controller itself faults, the e-stop is pressed, power is cut or the
          link to the actuator is lost, <Em>no retract command can reach the rod from
          software</Em>. The hardware e-stop and the hand switch are the only backstop in
          those cases. "Retract now" in this page needs a working connection too.
        </p>
      </Section>
    </div>
  )
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div className="flex flex-col gap-1">
      <h3 className="text-[11px] font-semibold uppercase tracking-wide text-slate-300">{title}</h3>
      <div>{children}</div>
    </div>
  )
}

function Em({ children }: { children: React.ReactNode }) {
  return <span className="text-slate-300">{children}</span>
}
