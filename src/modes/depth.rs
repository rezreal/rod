//! Depth Trainer — slow, multi-session training to hold and oscillate at
//! gradually increasing depth (SSCP extension, not part of Handy FW4).
//!
//! **Not tested on hardware.** See `docs/depth-trainer-design.md` for the design
//! and the safety rationale; the short version:
//!
//! * The rod moves **only while the hardware hand switch is held** (the deadman).
//!   Release, a stale status poll, an alarm/e-stop, a lost link, `Stop`, a
//!   transport drop or a mode switch all trigger an *emergency retract*: `Stop`,
//!   then a move back to the work-piece origin at the retract speed. The servo
//!   is then left energised (this unit has no working brake — see impale.rs).
//! * Time spent deeper than the user's breathing-safe offset is budgeted against
//!   a fraction of their recorded breath-hold, with an absolute compile-time
//!   ceiling ([`ABS_MAX_HOLD_S`]) that config cannot raise. A watchdog on the
//!   *sensed* position retracts before the budget is exhausted, whatever the plan
//!   said. Every block is followed by a mandatory rest.
//! * Depth is measured in mm beyond the calibrated origin and progresses slowly
//!   (small step, cooldown, only on an explicit "comfortable" rating).
//!
//! The module has three layers: [`Limits`]/[`Progress`] (pure planning and
//! persistence), [`Machine`] (a pure, time-injected state machine that turns
//! sensor samples into [`Action`]s — this is what the unit tests drive), and
//! [`DepthTask`] (the tokio task that samples `AppState`, executes the actions
//! and publishes telemetry).

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, RwLock};
use tokio::time::{Instant, MissedTickBehavior};
use tracing::{info, warn};

use super::{DepthControl, DepthRating};
use crate::config::{Config, DepthTrainer as DepthCfg, MotionProfile};
use crate::sscp::{DepthHistoryEntry, DepthProgressTelemetry};
use crate::state::{ActuatorCommand, AppMode, AppState, DepthPhase, DepthRuntime};

// ───────────────────────── compile-time safety bounds ─────────────────────────
// Config can move values *within* these bounds (more cautious), never outside.

/// Absolute ceiling for one excursion beyond the breathing-safe offset (s).
pub const ABS_MAX_HOLD_S: f32 = 20.0;
/// Largest fraction of the recorded breath-hold one excursion may use.
pub const ABS_MAX_FRACTION: f32 = 0.6;
/// Rest after a block is at least `MIN_REST_FACTOR ×` the time submerged …
pub const MIN_REST_FACTOR: f32 = 3.0;
/// … and never shorter than this (s).
pub const MIN_REST_S: f32 = 20.0;
/// Minimum pause at the breathing-safe turnaround between strokes (s).
pub const MIN_BREATH_WINDOW_S: f32 = 2.0;
/// Largest depth step per advance (mm).
pub const MAX_STEP_MM: f32 = 2.0;
/// Shortest allowed cooldown between depth increases (hours).
pub const MIN_COOLDOWN_H: f32 = 12.0;
/// Shallowest depth the trainer will plan (mm beyond origin).
pub const MIN_DEPTH_MM: f32 = 2.0;
/// Shortest dwell worth planning (s); below it the hold is refused.
pub const MIN_DWELL_S: f32 = 2.0;
/// Accepted range for the recorded breath-hold (s) and breathing-safe offset (mm).
pub const MIN_BREATH_HOLD_S: f32 = 5.0;
pub const MAX_BREATH_HOLD_S: f32 = 180.0;
pub const MAX_BREATH_OFFSET_MM: f32 = 100.0;

/// Smallest breathing-safe offset (mm) that leaves room for the timed warm-up.
const MIN_WARM_STROKE_MM: f32 = 2.0;
const MAX_SPEED_MM_S: f32 = 8.0;
const MAX_RETURN_SPEED_MM_S: f32 = 20.0;
const MAX_RETRACT_SPEED_MM_S: f32 = 40.0;
/// Mechanical clearance off the far hard stop (matches the driver's own margin).
const HARD_STOP_MARGIN_MM: f32 = 3.0;
/// Safety margin folded into every breath-budget calculation (s).
const MARGIN_S: f32 = 1.0;
/// Travel-time allowance for acceleration/deceleration ramps.
const ACCEL_FUDGE: f32 = 1.15;
/// Position tolerance for "reached the target" (mm).
const POS_TOL_MM: f32 = 0.8;
/// Hysteresis for "deeper than the breathing-safe offset" (mm).
const SUB_HYST_MM: f32 = 0.3;
/// Tick period of the task loop.
const TICK: Duration = Duration::from_millis(25);
/// After ServoOn the IAI controller needs ~50 ms before it accepts a move.
const SERVO_SETTLE: Duration = Duration::from_millis(50);
const HISTORY_KEEP: usize = 50;
const HISTORY_SHOWN: usize = 10;

fn fin(v: f32, lo: f32, hi: f32, dflt: f32) -> f32 {
    if v.is_finite() {
        v.clamp(lo, hi)
    } else {
        dflt
    }
}

// ───────────────────────────────── Limits ─────────────────────────────────

/// Sanitised, clamped view of the `[actuator.depth_trainer]` config.
#[derive(Debug, Clone)]
pub struct Limits {
    pub speed: f32,
    pub approach_speed: f32,
    pub approach_zone: f32,
    pub return_speed: f32,
    pub retract_speed: f32,
    pub accel_g: f32,
    pub retract_accel_g: f32,
    pub deadman_timeout: Duration,
    pub breath_fraction: f32,
    pub max_hold_s: f32,
    pub rest_factor: f32,
    pub min_rest_s: f32,
    pub breath_window_s: f32,
    pub step_mm: f32,
    pub cooldown_ms: u64,
    pub hold_step_s: f32,
    pub start_osc_mm: f32,
    pub start_hold_mm: f32,
    pub start_hold_s: f32,
    pub warmup_strokes: u32,
    pub warmup_s: f32,
    pub warmup_extend_s: f32,
    pub warmup_max_s: f32,
    pub osc_sets: u32,
    pub strokes_per_set: u32,
    pub hold_sets: u32,
    pub cooldown_strokes: u32,
    pub ready_countdown_s: f32,
    /// Absolute position (mm) no move may exceed.
    pub ceiling_abs_mm: f32,
    pub profile: MotionProfile,
    pub progress_path: PathBuf,
}

impl Limits {
    pub fn from_config(cfg: &Config) -> Self {
        Self::from_parts(
            &cfg.actuator.depth_trainer,
            cfg.stroke_mm(),
            cfg.max_position_mm(),
            cfg.motion_profile().unwrap_or(MotionProfile::SCurve),
        )
    }

    pub fn from_parts(
        c: &DepthCfg,
        stroke_mm: f32,
        max_position_mm: f32,
        profile: MotionProfile,
    ) -> Self {
        let speed = fin(c.speed_mm_s, 0.5, MAX_SPEED_MM_S, 4.0);
        let approach_speed = fin(c.approach_speed_mm_s, 0.3, speed, speed.min(1.6));
        // The emergency retract must always be noticeably faster than the advance.
        let retract_lo = (speed * 2.0).min(MAX_RETRACT_SPEED_MM_S);
        let retract_speed = fin(
            c.retract_speed_mm_s,
            retract_lo,
            MAX_RETRACT_SPEED_MM_S,
            20.0_f32.max(retract_lo),
        );
        let return_speed = fin(
            c.return_speed_mm_s,
            1.0,
            MAX_RETURN_SPEED_MM_S.min(retract_speed),
            10.0_f32.min(retract_speed),
        );
        Limits {
            speed,
            approach_speed,
            approach_zone: fin(c.approach_zone_mm, 1.0, 20.0, 5.0),
            return_speed,
            retract_speed,
            accel_g: fin(c.accel_g, 0.01, 0.1, 0.05),
            retract_accel_g: fin(c.retract_accel_g, 0.05, 0.3, 0.2),
            deadman_timeout: Duration::from_millis(c.deadman_timeout_ms.clamp(100, 300)),
            breath_fraction: fin(c.breath_fraction, 0.05, ABS_MAX_FRACTION, ABS_MAX_FRACTION),
            max_hold_s: fin(c.max_hold_s, 3.0, ABS_MAX_HOLD_S, ABS_MAX_HOLD_S),
            rest_factor: fin(c.rest_factor, MIN_REST_FACTOR, 20.0, MIN_REST_FACTOR),
            min_rest_s: fin(c.min_rest_s, MIN_REST_S, 600.0, MIN_REST_S),
            breath_window_s: fin(
                c.breath_window_s,
                MIN_BREATH_WINDOW_S,
                30.0,
                MIN_BREATH_WINDOW_S,
            ),
            step_mm: fin(c.step_mm, 0.5, MAX_STEP_MM, MAX_STEP_MM),
            cooldown_ms: (fin(c.advance_cooldown_h, MIN_COOLDOWN_H, 24.0 * 30.0, 24.0) as f64
                * 3_600_000.0) as u64,
            hold_step_s: fin(c.hold_step_s, 0.5, 5.0, 1.0),
            start_osc_mm: fin(c.start_oscillation_depth_mm, MIN_DEPTH_MM, 50.0, 10.0),
            start_hold_mm: fin(c.start_hold_depth_mm, MIN_DEPTH_MM, 50.0, 5.0),
            start_hold_s: fin(c.start_hold_s, MIN_DWELL_S, ABS_MAX_HOLD_S, 5.0),
            warmup_strokes: c.warmup_strokes.clamp(1, 10),
            warmup_s: fin(c.warmup_s, 30.0, 600.0, 120.0),
            warmup_extend_s: fin(c.warmup_extend_s, 15.0, 300.0, 60.0),
            warmup_max_s: fin(c.warmup_max_s, 30.0, 900.0, 600.0)
                .max(fin(c.warmup_s, 30.0, 600.0, 120.0)),
            osc_sets: c.oscillation_sets.clamp(1, 6),
            strokes_per_set: c.strokes_per_set.clamp(1, 12),
            hold_sets: c.hold_sets.clamp(1, 6),
            cooldown_strokes: c.cooldown_strokes.clamp(1, 6),
            ready_countdown_s: fin(c.ready_countdown_s, 1.0, 10.0, 3.0),
            ceiling_abs_mm: max_position_mm.min(stroke_mm - HARD_STOP_MARGIN_MM),
            profile,
            progress_path: PathBuf::from(&c.progress_file),
        }
    }

    /// Per-excursion cap (s) for a recorded breath-hold.
    pub fn cap_s(&self, breath_hold_s: f32) -> f32 {
        (breath_hold_s * self.breath_fraction)
            .min(self.max_hold_s)
            .max(0.0)
    }

    /// Mandatory rest (s) after a block that kept the user submerged for `submerged_s`.
    pub fn rest_s(&self, submerged_s: f32) -> f32 {
        (self.rest_factor * submerged_s).max(self.min_rest_s)
    }

    /// Estimated outbound time covering `beyond_mm` past the breathing-safe offset.
    pub fn out_time_s(&self, beyond_mm: f32) -> f32 {
        let slow = beyond_mm.min(self.approach_zone).max(0.0);
        let fast = (beyond_mm - slow).max(0.0);
        (fast / self.speed + slow / self.approach_speed) * ACCEL_FUDGE
    }

    /// Estimated normal-return time over `beyond_mm`.
    pub fn back_time_s(&self, beyond_mm: f32) -> f32 {
        beyond_mm.max(0.0) / self.return_speed * ACCEL_FUDGE
    }

    /// Estimated emergency-retract time over `beyond_mm` (plus a latency allowance).
    pub fn emergency_time_s(&self, beyond_mm: f32) -> f32 {
        beyond_mm.max(0.0) / self.retract_speed * ACCEL_FUDGE + 0.3
    }

    /// Dwell (s) a hold at `depth_mm` may last inside `cap_s`: the cap minus the
    /// outbound and return transits (both count as submerged) minus a margin.
    pub fn dwell_budget_s(&self, cap_s: f32, depth_mm: f32, offset_mm: f32) -> f32 {
        let b = (depth_mm - offset_mm).max(0.0);
        cap_s - self.out_time_s(b) - self.back_time_s(b) - MARGIN_S
    }

    /// Submerged time of one stroke (out + back, no dwell).
    pub fn stroke_sub_time_s(&self, depth_mm: f32, offset_mm: f32) -> f32 {
        let b = (depth_mm - offset_mm).max(0.0);
        self.out_time_s(b) + self.back_time_s(b)
    }

    /// Strokes one oscillation set may contain so its *cumulative* submerged
    /// time stays inside the cap.
    pub fn strokes_in_set(&self, cap_s: f32, depth_mm: f32, offset_mm: f32) -> u32 {
        let t = self.stroke_sub_time_s(depth_mm, offset_mm);
        if t <= 0.0 {
            return self.strokes_per_set;
        }
        (((cap_s - MARGIN_S) / t).floor().max(0.0) as u32).min(self.strokes_per_set)
    }
}

// ───────────────────────────── planning ─────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BlockKind {
    /// Short shallow strokes (used when there is no breathing-safe room).
    Warmup,
    /// Timed gentle strokes inside the breathing-safe offset; `dwell_s` is the
    /// duration. Never submerges, so no breath budget or rest applies.
    WarmTimed,
    Oscillation,
    Hold,
    Cooldown,
}

/// One unit of a session: a run of strokes or a single hold.
#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub kind: BlockKind,
    pub depth_mm: f32,
    /// Strokes in this block (0 for a hold).
    pub strokes: u32,
    /// Planned dwell (s) for a hold.
    pub dwell_s: f32,
    pub set_idx: u32,
    pub set_count: u32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Targets {
    pub osc_depth_mm: f32,
    pub hold_depth_mm: f32,
    pub hold_s: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PlanError {
    NoBreathHold,
    CapTooShort(f32),
    NoDepthRoom,
    OscillationUnreachable,
    HoldUnreachable { depth_mm: f32, budget_s: f32 },
}

impl std::fmt::Display for PlanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlanError::NoBreathHold => write!(f, "set your breath-hold time first"),
            PlanError::CapTooShort(c) => {
                write!(f, "breath-hold limit too short to train safely ({c:.1} s)")
            }
            PlanError::NoDepthRoom => {
                write!(f, "work-piece origin leaves no room below the depth limit")
            }
            PlanError::OscillationUnreachable => write!(
                f,
                "oscillation depth can't be reached inside the breath-hold limit — lower the depth or raise the breath-hold"
            ),
            PlanError::HoldUnreachable { depth_mm, budget_s } => write!(
                f,
                "hold at {depth_mm:.0} mm leaves only {budget_s:.1} s inside the breath-hold limit — step back or raise the breath-hold"
            ),
        }
    }
}

/// Everything the planner needs to know about the machine right now.
#[derive(Debug, Clone, Copy)]
pub struct StartCtx {
    pub idle: bool,
    pub connected: bool,
    pub faulted: bool,
    pub calibrating: bool,
    pub origin: Option<f32>,
    pub max_depth_mm: f32,
}

impl StartCtx {
    pub fn from_state(st: &AppState) -> Self {
        StartCtx {
            idle: st.mode == AppMode::Idle,
            connected: st.actuator_connected,
            faulted: st.alarm_major || st.emergency_stop,
            calibrating: st.calibrating,
            origin: st.work_origin_mm,
            max_depth_mm: st.max_depth_mm,
        }
    }
}

/// Build the block list for one session. Pure; refuses (never shortens
/// silently) a plan the breath budget can't support.
pub fn plan_session(
    lim: &Limits,
    targets: &Targets,
    cap_s: f32,
    offset_mm: f32,
) -> Result<(Vec<Block>, Targets), PlanError> {
    if cap_s < 4.0 {
        return Err(PlanError::CapTooShort(cap_s));
    }
    let n_osc = lim.strokes_in_set(cap_s, targets.osc_depth_mm, offset_mm);
    if n_osc == 0 {
        return Err(PlanError::OscillationUnreachable);
    }
    let budget = lim.dwell_budget_s(cap_s, targets.hold_depth_mm, offset_mm);
    if budget < MIN_DWELL_S {
        return Err(PlanError::HoldUnreachable {
            depth_mm: targets.hold_depth_mm,
            budget_s: budget,
        });
    }
    let dwell = targets.hold_s.min(budget).max(MIN_DWELL_S);
    let mut blocks = Vec::new();
    if offset_mm >= MIN_WARM_STROKE_MM {
        // Gentle timed warm-up that stays above the breathing-safe offset.
        blocks.push(Block {
            kind: BlockKind::WarmTimed,
            depth_mm: offset_mm.min((targets.osc_depth_mm * 0.5).max(MIN_DEPTH_MM)),
            strokes: 0,
            dwell_s: lim.warmup_s,
            set_idx: 1,
            set_count: 1,
        });
    } else {
        blocks.push(Block {
            kind: BlockKind::Warmup,
            depth_mm: (targets.osc_depth_mm * 0.5).max(MIN_DEPTH_MM),
            strokes: lim.warmup_strokes,
            dwell_s: 0.0,
            set_idx: 1,
            set_count: 1,
        });
    }
    for i in 0..lim.osc_sets {
        blocks.push(Block {
            kind: BlockKind::Oscillation,
            depth_mm: targets.osc_depth_mm,
            strokes: n_osc,
            dwell_s: 0.0,
            set_idx: i + 1,
            set_count: lim.osc_sets,
        });
    }
    for i in 0..lim.hold_sets {
        blocks.push(Block {
            kind: BlockKind::Hold,
            depth_mm: targets.hold_depth_mm,
            strokes: 0,
            dwell_s: dwell,
            set_idx: i + 1,
            set_count: lim.hold_sets,
        });
    }
    blocks.push(Block {
        kind: BlockKind::Cooldown,
        depth_mm: (targets.osc_depth_mm * 0.3).max(MIN_DEPTH_MM),
        strokes: lim.cooldown_strokes,
        dwell_s: 0.0,
        set_idx: 1,
        set_count: 1,
    });
    Ok((
        blocks,
        Targets {
            hold_s: dwell,
            ..*targets
        },
    ))
}

// ─────────────────────────────── progress ───────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionRecord {
    pub at_ms: u64,
    pub osc_depth_mm: f32,
    pub hold_depth_mm: f32,
    pub hold_s: f32,
    pub early_releases: u32,
    pub rating: Option<String>,
}

/// What happened in a session (for progression).
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub early_releases: u32,
    /// Why the session was cut short by a safety event, if it was.
    pub emergency: Option<String>,
    /// Every block ran to completion.
    pub completed: bool,
}

impl Outcome {
    fn clean(&self) -> bool {
        self.early_releases == 0 && self.emergency.is_none() && self.completed
    }
}

/// Persisted training progress. `Default` is the most conservative state.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Progress {
    /// Recorded comfortable breath-hold (s). Required before the first session.
    pub breath_hold_s: Option<f32>,
    /// Depth (mm beyond origin) down to which the user can still breathe.
    pub breath_offset_mm: f32,
    /// Last depth rated comfortable for oscillation.
    pub osc_comfortable_mm: Option<f32>,
    /// Last hold (depth, dwell) rated comfortable.
    pub hold_comfortable_mm: Option<f32>,
    pub hold_comfortable_s: Option<f32>,
    /// Epoch ms of the last comfortable rating / step back (starts the cooldown).
    pub last_progress_ms: Option<u64>,
    pub sessions: u32,
    pub early_release_streak: u32,
    pub history: Vec<SessionRecord>,
}

impl Progress {
    /// Load from `path`. Missing → default. Corrupt or out-of-range → the
    /// offending value is dropped (never replaced by something larger).
    pub fn load(path: &Path) -> Progress {
        match std::fs::read_to_string(path) {
            Ok(s) => match serde_json::from_str::<Progress>(&s) {
                Ok(p) => p.sanitized(),
                Err(e) => {
                    warn!(error = %e, path = %path.display(), "depth-trainer progress unreadable; starting from conservative defaults");
                    let _ = std::fs::rename(path, path.with_extension("corrupt"));
                    Progress::default()
                }
            },
            Err(_) => Progress::default(),
        }
    }

    /// Atomic save (tmp + rename).
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let tmp = path.with_extension("tmp");
        std::fs::write(
            &tmp,
            serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?,
        )?;
        std::fs::rename(&tmp, path)
    }

    fn sanitized(mut self) -> Progress {
        let ok = |v: f32, lo: f32, hi: f32| v.is_finite() && v >= lo && v <= hi;
        self.breath_hold_s = self
            .breath_hold_s
            .filter(|v| ok(*v, MIN_BREATH_HOLD_S, MAX_BREATH_HOLD_S));
        if !ok(self.breath_offset_mm, 0.0, MAX_BREATH_OFFSET_MM) {
            self.breath_offset_mm = 0.0;
        }
        self.osc_comfortable_mm = self
            .osc_comfortable_mm
            .filter(|v| ok(*v, MIN_DEPTH_MM, 300.0));
        let pair_ok = matches!(
            (self.hold_comfortable_mm, self.hold_comfortable_s),
            (Some(d), Some(s)) if ok(d, MIN_DEPTH_MM, 300.0) && ok(s, MIN_DWELL_S, ABS_MAX_HOLD_S)
        );
        if !pair_ok {
            self.hold_comfortable_mm = None;
            self.hold_comfortable_s = None;
        }
        let keep = self.history.len().saturating_sub(HISTORY_KEEP);
        self.history.drain(..keep);
        self
    }

    pub fn set_settings(
        &mut self,
        breath_hold_s: f32,
        breath_offset_mm: f32,
    ) -> Result<(), String> {
        if !(breath_hold_s.is_finite()
            && (MIN_BREATH_HOLD_S..=MAX_BREATH_HOLD_S).contains(&breath_hold_s))
        {
            return Err(format!(
                "breath-hold must be {MIN_BREATH_HOLD_S:.0}–{MAX_BREATH_HOLD_S:.0} s"
            ));
        }
        if !(breath_offset_mm.is_finite()
            && (0.0..=MAX_BREATH_OFFSET_MM).contains(&breath_offset_mm))
        {
            return Err(format!(
                "breathing-safe offset must be 0–{MAX_BREATH_OFFSET_MM:.0} mm"
            ));
        }
        self.breath_hold_s = Some(breath_hold_s);
        self.breath_offset_mm = breath_offset_mm;
        Ok(())
    }

    fn advance_allowed(&self, lim: &Limits, now_ms: u64) -> bool {
        match self.last_progress_ms {
            None => true,
            Some(l) => now_ms.saturating_sub(l) >= lim.cooldown_ms,
        }
    }

    /// Seconds until a depth increase is allowed again (0 = now).
    pub fn next_advance_in_s(&self, lim: &Limits, now_ms: u64) -> f32 {
        if self.osc_comfortable_mm.is_none() {
            return 0.0;
        }
        match self.last_progress_ms {
            None => 0.0,
            Some(l) => (lim.cooldown_ms.saturating_sub(now_ms.saturating_sub(l))) as f32 / 1000.0,
        }
    }

    /// Targets for the next session: stored comfortable values plus at most one
    /// step, and only once the cooldown has elapsed. Clamped to `ceiling_mm`.
    pub fn next_targets(
        &self,
        lim: &Limits,
        now_ms: u64,
        ceiling_mm: f32,
        cap_s: f32,
        offset_mm: f32,
    ) -> Targets {
        let adv = self.advance_allowed(lim, now_ms);
        let osc = match self.osc_comfortable_mm {
            None => lim.start_osc_mm,
            Some(c) if adv => c + lim.step_mm,
            Some(c) => c,
        };
        let (hd, hs) = match (self.hold_comfortable_mm, self.hold_comfortable_s) {
            (Some(d), Some(s)) if adv => {
                let budget = lim.dwell_budget_s(cap_s, d, offset_mm);
                if s + lim.hold_step_s <= budget {
                    (d, s + lim.hold_step_s)
                } else {
                    (d + lim.step_mm, lim.start_hold_s)
                }
            }
            (Some(d), Some(s)) => (d, s),
            _ => (lim.start_hold_mm, lim.start_hold_s),
        };
        let hi = ceiling_mm.max(MIN_DEPTH_MM);
        Targets {
            osc_depth_mm: osc.clamp(MIN_DEPTH_MM, hi),
            hold_depth_mm: hd.clamp(MIN_DEPTH_MM, hi),
            hold_s: hs,
        }
    }

    fn ceiling_mm(lim: &Limits, ctx: &StartCtx, origin: f32) -> f32 {
        let abs = if ctx.max_depth_mm > 0.0 {
            lim.ceiling_abs_mm.min(ctx.max_depth_mm)
        } else {
            lim.ceiling_abs_mm
        };
        abs - origin
    }

    /// Everything needed to begin a session, or the reason it can't begin.
    pub fn can_start(
        &self,
        lim: &Limits,
        ctx: &StartCtx,
        now_ms: u64,
    ) -> Result<(Targets, f32, f32, Vec<Block>), String> {
        if !ctx.idle {
            return Err("another program is running".into());
        }
        if !ctx.connected {
            return Err("actuator not connected".into());
        }
        if ctx.faulted {
            return Err("controller alarm or emergency stop active".into());
        }
        if ctx.calibrating {
            return Err("calibration in progress".into());
        }
        let origin = ctx
            .origin
            .ok_or_else(|| "not calibrated — run calibration first".to_string())?;
        let breath = self
            .breath_hold_s
            .ok_or_else(|| PlanError::NoBreathHold.to_string())?;
        let ceiling = Self::ceiling_mm(lim, ctx, origin);
        if ceiling < MIN_DEPTH_MM {
            return Err(PlanError::NoDepthRoom.to_string());
        }
        let cap = lim.cap_s(breath);
        let offset = self.breath_offset_mm;
        let want = self.next_targets(lim, now_ms, ceiling, cap, offset);
        let (blocks, eff) = plan_session(lim, &want, cap, offset).map_err(|e| e.to_string())?;
        Ok((eff, cap, origin, blocks))
    }

    fn push_history(&mut self, used: &Targets, o: &Outcome, rating: Option<String>, now_ms: u64) {
        self.history.push(SessionRecord {
            at_ms: now_ms,
            osc_depth_mm: used.osc_depth_mm,
            hold_depth_mm: used.hold_depth_mm,
            hold_s: used.hold_s,
            early_releases: o.early_releases,
            rating,
        });
        let drop = self.history.len().saturating_sub(HISTORY_KEEP);
        self.history.drain(..drop);
    }

    fn streak(&mut self, o: &Outcome) {
        if o.early_releases > 0 {
            self.early_release_streak += 1;
        } else if o.clean() {
            self.early_release_streak = 0;
        }
    }

    /// A session ended without a rating (stopped, link lost, …): history only.
    pub fn record_unrated(&mut self, used: &Targets, o: &Outcome, now_ms: u64) {
        self.streak(o);
        self.push_history(used, o, None, now_ms);
    }

    /// Apply the user's rating. Only a clean session rated comfortable advances.
    pub fn apply_rating(
        &mut self,
        rating: DepthRating,
        used: &Targets,
        o: &Outcome,
        lim: &Limits,
        now_ms: u64,
    ) {
        self.sessions += 1;
        self.streak(o);
        let effective = match (rating, o.clean()) {
            (DepthRating::Comfortable, false) => DepthRating::Okay,
            (r, _) => r,
        };
        match effective {
            DepthRating::Comfortable => {
                self.osc_comfortable_mm = Some(used.osc_depth_mm);
                self.hold_comfortable_mm = Some(used.hold_depth_mm);
                self.hold_comfortable_s = Some(used.hold_s);
                self.last_progress_ms = Some(now_ms);
            }
            DepthRating::Okay => {}
            DepthRating::TooMuch => {
                // If the session was at (or below) what is already stored there
                // was no advance attempt to blame, so step the stored values back;
                // either way restart the cooldown.
                let at_stored = self
                    .osc_comfortable_mm
                    .map_or(true, |c| used.osc_depth_mm <= c + 0.01);
                if at_stored {
                    let (od, hd, hs) = Self::stepped_back(lim, used);
                    self.osc_comfortable_mm = Some(od);
                    self.hold_comfortable_mm = Some(hd);
                    self.hold_comfortable_s = Some(hs);
                }
                self.last_progress_ms = Some(now_ms);
            }
        }
        let label = match rating {
            DepthRating::Comfortable => "comfortable",
            DepthRating::Okay => "okay",
            DepthRating::TooMuch => "too_much",
        };
        self.push_history(used, o, Some(label.into()), now_ms);
    }

    fn stepped_back(lim: &Limits, t: &Targets) -> (f32, f32, f32) {
        let od = (t.osc_depth_mm - lim.step_mm).max(MIN_DEPTH_MM);
        let (hd, hs) = if t.hold_s - lim.hold_step_s >= MIN_DWELL_S {
            (t.hold_depth_mm, t.hold_s - lim.hold_step_s)
        } else {
            (
                (t.hold_depth_mm - lim.step_mm).max(MIN_DEPTH_MM),
                lim.start_hold_s,
            )
        };
        (od, hd, hs)
    }

    /// User-confirmed manual step back of the stored comfortable values.
    pub fn step_back(&mut self, lim: &Limits, now_ms: u64) {
        let cur = Targets {
            osc_depth_mm: self.osc_comfortable_mm.unwrap_or(lim.start_osc_mm),
            hold_depth_mm: self.hold_comfortable_mm.unwrap_or(lim.start_hold_mm),
            hold_s: self.hold_comfortable_s.unwrap_or(lim.start_hold_s),
        };
        let (od, hd, hs) = Self::stepped_back(lim, &cur);
        self.osc_comfortable_mm = Some(od);
        self.hold_comfortable_mm = Some(hd);
        self.hold_comfortable_s = Some(hs);
        self.last_progress_ms = Some(now_ms);
        self.early_release_streak = 0;
    }

    pub fn suggest_step_back(&self) -> bool {
        self.early_release_streak >= 2
    }

    /// Telemetry view (always sent, so the UI can show it before a session).
    pub fn view(&self, lim: &Limits, ctx: &StartCtx, now_ms: u64) -> DepthProgressTelemetry {
        let cap = self.breath_hold_s.map(|b| lim.cap_s(b));
        let ceiling = ctx
            .origin
            .map(|o| Self::ceiling_mm(lim, ctx, o))
            .unwrap_or(lim.ceiling_abs_mm);
        let t = self.next_targets(
            lim,
            now_ms,
            ceiling,
            cap.unwrap_or(0.0),
            self.breath_offset_mm,
        );
        let (can_start, block_reason) = match self.can_start(lim, ctx, now_ms) {
            Ok(_) => (true, None),
            Err(e) => (false, Some(e)),
        };
        let skip = self.history.len().saturating_sub(HISTORY_SHOWN);
        DepthProgressTelemetry {
            breath_hold_s: self.breath_hold_s,
            breath_offset_mm: self.breath_offset_mm,
            oscillation_depth_mm: t.osc_depth_mm,
            hold_depth_mm: t.hold_depth_mm,
            hold_s: t.hold_s,
            hold_cap_s: cap,
            sessions: self.sessions,
            next_advance_in_s: self.next_advance_in_s(lim, now_ms),
            suggest_step_back: self.suggest_step_back(),
            can_start,
            block_reason,
            history: self.history[skip..]
                .iter()
                .map(|r| DepthHistoryEntry {
                    at_ms: r.at_ms,
                    oscillation_depth_mm: r.osc_depth_mm,
                    hold_depth_mm: r.hold_depth_mm,
                    hold_s: r.hold_s,
                    early_releases: r.early_releases,
                    rating: r.rating.clone(),
                })
                .collect(),
        }
    }
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ───────────────────────────── state machine ─────────────────────────────

/// What the machine asks the executor to do.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Deceleration stop (servo stays on).
    Stop,
    MoveTo {
        pos_mm: f32,
        vel_mm_s: f32,
        accel_g: f32,
        profile: MotionProfile,
    },
}

/// One reading of the world, taken by the executor each tick.
#[derive(Debug, Clone, Copy)]
pub struct Sample {
    /// Sensed absolute position (mm).
    pub position_mm: f32,
    /// Hand switch held **and** its reading fresh and trustworthy.
    pub held: bool,
    /// A condition that ends the session outright (alarm, e-stop, link, stale data).
    pub fault: Option<&'static str>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Step {
    /// At/outside the origin: resting, or waiting for the switch.
    Idle,
    /// Timed warm-up: gentle stroke outward (to the warm-up depth).
    WarmOut {
        deadline: f64,
    },
    /// Timed warm-up: gentle stroke back to the origin.
    WarmBack {
        deadline: f64,
    },
    /// Warm-up finished; rod at the origin, waiting for Continue / + more.
    WarmGate,
    /// Switch held, counting down before the next advance.
    Ready {
        until: f64,
    },
    /// Moving to the origin before the first block (under deadman).
    Position {
        deadline: f64,
    },
    OutFast {
        deadline: f64,
    },
    OutSlow {
        deadline: f64,
    },
    Dwell {
        until: f64,
    },
    Back {
        deadline: f64,
    },
    /// Pause at the breathing-safe turnaround between strokes.
    Breath {
        until: f64,
    },
    /// Normal end-of-block return to the origin.
    Return {
        deadline: f64,
    },
    Emergency {
        deadline: f64,
        end: bool,
    },
    Done,
}

pub struct Machine {
    lim: Limits,
    origin: f32,
    offset: f32,
    cap_s: f32,
    blocks: VecDeque<Block>,
    cur: Option<Block>,
    step: Step,
    phase: DepthPhase,
    last_t: f64,
    rest_until: f64,
    rest_total: f32,
    sub_block_s: f32,
    stroke_idx: u32,
    leg_target: f32,
    /// End of the current timed warm-up block (machine time).
    warm_until: f64,
    /// Depth (mm) of the timed warm-up strokes, reused by extensions.
    warm_depth: f32,
    /// Sum of all warm-up time started so far, extensions included (s).
    warm_total_s: f32,
    /// A normal end-of-block return was upgraded to retract speed because the
    /// switch was released while it was still under way.
    return_fast: bool,
    early_releases: u32,
    emergency: Option<String>,
    end_reason: Option<String>,
    exit_requested: bool,
    completed: bool,
    // for telemetry
    held: bool,
    depth_beyond: f32,
    awaiting: bool,
    cue: Option<f32>,
}

impl Machine {
    pub fn new(
        lim: Limits,
        origin: f32,
        offset: f32,
        cap_s: f32,
        blocks: Vec<Block>,
        t0: f64,
    ) -> Self {
        Machine {
            lim,
            origin,
            offset,
            cap_s,
            blocks: blocks.into(),
            cur: None,
            step: Step::Idle,
            phase: DepthPhase::Arming,
            last_t: t0,
            rest_until: t0,
            rest_total: 0.0,
            sub_block_s: 0.0,
            stroke_idx: 0,
            leg_target: origin,
            warm_until: t0,
            warm_depth: 0.0,
            warm_total_s: 0.0,
            return_fast: false,
            early_releases: 0,
            emergency: None,
            end_reason: None,
            exit_requested: false,
            completed: false,
            held: false,
            depth_beyond: 0.0,
            awaiting: false,
            cue: None,
        }
    }

    pub fn is_done(&self) -> bool {
        matches!(self.step, Step::Done)
    }

    pub fn exit_requested(&self) -> bool {
        self.exit_requested
    }

    pub fn outcome(&self) -> Outcome {
        Outcome {
            early_releases: self.early_releases,
            emergency: self.end_reason.clone(),
            completed: self.completed && self.end_reason.is_none(),
        }
    }

    fn mv(&self, pos_abs: f32, vel: f32, accel: f32) -> Action {
        Action::MoveTo {
            pos_mm: pos_abs,
            vel_mm_s: vel,
            accel_g: accel,
            profile: self.lim.profile,
        }
    }

    fn at_risk(&self) -> bool {
        matches!(
            self.step,
            Step::Position { .. }
                | Step::OutFast { .. }
                | Step::OutSlow { .. }
                | Step::Dwell { .. }
                | Step::Back { .. }
                | Step::Breath { .. }
                | Step::WarmOut { .. }
                | Step::WarmBack { .. }
        )
    }

    /// Steps during which the breath watchdog applies.
    fn watched(&self) -> bool {
        self.at_risk() || matches!(self.step, Step::Return { .. })
    }

    /// Turnaround position (abs) of an oscillation stroke: the breathing-safe
    /// offset, kept at least 2 mm short of the stroke's depth.
    fn turnaround_abs(&self, depth_mm: f32) -> f32 {
        self.origin + self.offset.min((depth_mm - 2.0).max(0.0))
    }

    /// Run one tick. Returns the actions to execute, in order.
    pub fn tick(&mut self, t: f64, s: &Sample) -> Vec<Action> {
        let dt = (t - self.last_t).max(0.0) as f32;
        self.last_t = t;
        let pos = s.position_mm;
        self.depth_beyond = (pos - self.origin).max(0.0);
        self.held = s.held;
        if self.is_done() {
            return Vec::new();
        }
        let past_offset = (pos - (self.origin + self.offset)).max(0.0);
        if past_offset > SUB_HYST_MM {
            self.sub_block_s += dt;
        }

        let mut actions = Vec::new();
        if let Some(reason) = s.fault {
            actions.extend(self.emergency(t, reason, true));
        } else if !s.held && self.at_risk() {
            self.early_releases += 1;
            let end = self.early_releases >= 2;
            actions.extend(self.emergency(t, "hand switch released", end));
        } else if !s.held && matches!(self.step, Step::Return { .. }) && !self.return_fast {
            // Already heading home, but a released switch means "get out now":
            // finish the return at retract speed (not an early release).
            self.return_fast = true;
            actions.push(self.mv(
                self.origin,
                self.lim.retract_speed,
                self.lim.retract_accel_g,
            ));
        } else if self.watched()
            && self.cap_s > 0.0
            && self.sub_block_s + self.lim.emergency_time_s(past_offset) + MARGIN_S >= self.cap_s
        {
            actions.extend(self.emergency(t, "breath-hold limit reached", true));
        }
        actions.extend(self.advance(t, pos, s.held));
        actions
    }

    /// `Stop` / mode switch / transport drop: retract (if needed) and exit.
    pub fn stop(&mut self, t: f64) -> Vec<Action> {
        self.exit_requested = true;
        match self.step {
            Step::Done => Vec::new(),
            Step::Emergency { deadline, .. } => {
                self.step = Step::Emergency {
                    deadline,
                    end: true,
                };
                Vec::new()
            }
            Step::Idle | Step::Ready { .. } | Step::WarmGate if self.depth_beyond <= POS_TOL_MM => {
                self.step = Step::Done;
                self.phase = DepthPhase::Ended;
                Vec::new()
            }
            _ => self.emergency(t, "stopped", true),
        }
    }

    fn emergency(&mut self, t: f64, reason: &str, end: bool) -> Vec<Action> {
        if let Step::Emergency {
            deadline,
            end: was_end,
        } = self.step
        {
            self.step = Step::Emergency {
                deadline,
                end: was_end || end,
            };
            if end && self.end_reason.is_none() {
                self.end_reason = Some(reason.to_string());
            }
            return Vec::new();
        }
        let est = self.lim.emergency_time_s(self.depth_beyond);
        self.step = Step::Emergency {
            deadline: t + (est * 2.0 + 3.0) as f64,
            end,
        };
        self.phase = DepthPhase::Retracting;
        self.emergency = Some(reason.to_string());
        if end {
            self.end_reason = Some(reason.to_string());
        }
        self.cue = None;
        vec![
            Action::Stop,
            self.mv(
                self.origin,
                self.lim.retract_speed,
                self.lim.retract_accel_g,
            ),
        ]
    }

    fn begin_rest(&mut self, t: f64) {
        self.rest_until = t + self.rest_total as f64;
        self.step = Step::Idle;
    }

    fn finish_done(&mut self) {
        self.step = Step::Done;
        self.phase = DepthPhase::Rating;
        self.completed = self.end_reason.is_none() && !self.exit_requested;
    }

    fn begin_return(&mut self, t: f64, pos: f32) -> Vec<Action> {
        let dist = (pos - self.origin).abs();
        self.return_fast = false;
        self.step = Step::Return {
            deadline: t + (2.0 * dist / self.lim.return_speed + 3.0) as f64,
        };
        self.phase = DepthPhase::Retracting;
        vec![self.mv(self.origin, self.lim.return_speed, self.lim.accel_g)]
    }

    fn finish_block(&mut self, t: f64, pos: f32) -> Vec<Action> {
        self.rest_total = if self.blocks.is_empty() {
            0.0
        } else {
            self.lim.rest_s(self.sub_block_s)
        };
        self.cur = None;
        if pos > self.origin + POS_TOL_MM {
            self.begin_return(t, pos)
        } else {
            self.begin_rest(t);
            Vec::new()
        }
    }

    fn start_block(&mut self, t: f64, pos: f32) -> Vec<Action> {
        let Some(b) = self.blocks.pop_front() else {
            self.finish_done();
            return Vec::new();
        };
        self.sub_block_s = 0.0;
        self.stroke_idx = 0;
        self.phase = match b.kind {
            BlockKind::Warmup | BlockKind::WarmTimed => DepthPhase::Warmup,
            BlockKind::Oscillation => DepthPhase::Oscillation,
            BlockKind::Hold => DepthPhase::Hold,
            BlockKind::Cooldown => DepthPhase::Cooldown,
        };
        if b.kind == BlockKind::WarmTimed {
            self.warm_until = t + b.dwell_s as f64;
            self.warm_depth = b.depth_mm;
            self.warm_total_s += b.dwell_s;
            self.cur = Some(b);
            return self.begin_warm_out(t, pos);
        }
        self.cur = Some(b);
        self.begin_out(t, pos)
    }

    fn begin_warm_out(&mut self, t: f64, pos: f32) -> Vec<Action> {
        let target = self.origin + self.warm_depth;
        self.leg_target = target;
        let d = (target - pos).abs();
        self.step = Step::WarmOut {
            deadline: t + (2.0 * d / self.lim.speed + 3.0) as f64,
        };
        vec![self.mv(target, self.lim.speed, self.lim.accel_g)]
    }

    fn begin_warm_back(&mut self, t: f64, pos: f32) -> Vec<Action> {
        self.leg_target = self.origin;
        let d = (pos - self.origin).abs();
        self.step = Step::WarmBack {
            deadline: t + (2.0 * d / self.lim.speed + 3.0) as f64,
        };
        vec![self.mv(self.origin, self.lim.speed, self.lim.accel_g)]
    }

    /// The user is ready for the main blocks.
    pub fn warmup_continue(&mut self) -> bool {
        if matches!(self.step, Step::WarmGate) {
            self.step = Step::Idle;
            true
        } else {
            false
        }
    }

    /// Add another warm-up period (the next advance still needs the switch held
    /// and the usual countdown). Refused past the total warm-up ceiling.
    pub fn warmup_extend(&mut self) -> bool {
        if !matches!(self.step, Step::WarmGate) || !self.can_extend_warmup() {
            return false;
        }
        self.blocks.push_front(Block {
            kind: BlockKind::WarmTimed,
            depth_mm: self.warm_depth,
            strokes: 0,
            dwell_s: self.lim.warmup_extend_s,
            set_idx: 1,
            set_count: 1,
        });
        self.step = Step::Idle;
        true
    }

    fn can_extend_warmup(&self) -> bool {
        self.warm_total_s + self.lim.warmup_extend_s <= self.lim.warmup_max_s + 1e-3
    }

    fn begin_out(&mut self, t: f64, pos: f32) -> Vec<Action> {
        let depth = self.cur.as_ref().map(|b| b.depth_mm).unwrap_or(0.0);
        let target = self.origin + depth;
        let fast_target = target - self.lim.approach_zone;
        if fast_target > pos + 1.0 {
            self.leg_target = fast_target;
            let d = fast_target - pos;
            self.step = Step::OutFast {
                deadline: t + (2.0 * d / self.lim.speed + 3.0) as f64,
            };
            vec![self.mv(fast_target, self.lim.speed, self.lim.accel_g)]
        } else {
            self.begin_out_slow(t, pos)
        }
    }

    fn begin_out_slow(&mut self, t: f64, pos: f32) -> Vec<Action> {
        let depth = self.cur.as_ref().map(|b| b.depth_mm).unwrap_or(0.0);
        let target = self.origin + depth;
        self.leg_target = target;
        let d = (target - pos).max(0.0);
        self.step = Step::OutSlow {
            deadline: t + (2.0 * d / self.lim.approach_speed + 3.0) as f64,
        };
        vec![self.mv(target, self.lim.approach_speed, self.lim.accel_g)]
    }

    fn begin_back(&mut self, t: f64, pos: f32) -> Vec<Action> {
        let b = self.cur.clone().expect("back without a block");
        let target = if b.kind == BlockKind::Hold {
            self.origin
        } else {
            self.turnaround_abs(b.depth_mm)
        };
        self.leg_target = target;
        let d = (pos - target).abs();
        self.step = Step::Back {
            deadline: t + (2.0 * d / self.lim.return_speed + 3.0) as f64,
        };
        vec![self.mv(target, self.lim.return_speed, self.lim.accel_g)]
    }

    fn reached(pos: f32, target: f32) -> bool {
        (pos - target).abs() <= POS_TOL_MM
    }

    fn advance(&mut self, t: f64, pos: f32, held: bool) -> Vec<Action> {
        self.cue = None;
        self.awaiting = false;
        match self.step {
            Step::Done => Vec::new(),

            Step::Idle => {
                if t < self.rest_until {
                    self.phase = DepthPhase::Rest;
                    return Vec::new();
                }
                self.rest_total = 0.0;
                if self.blocks.is_empty() && self.cur.is_none() {
                    self.finish_done();
                    return Vec::new();
                }
                self.phase = DepthPhase::Arming;
                if held {
                    self.step = Step::Ready {
                        until: t + self.lim.ready_countdown_s as f64,
                    };
                } else {
                    self.awaiting = true;
                }
                Vec::new()
            }

            Step::Ready { until } => {
                if !held {
                    self.step = Step::Idle;
                    self.awaiting = true;
                    return Vec::new();
                }
                self.cue = Some((until - t).max(0.0) as f32);
                if t >= until {
                    if (pos - self.origin).abs() > POS_TOL_MM {
                        let dist = (pos - self.origin).abs();
                        self.step = Step::Position {
                            deadline: t + (2.0 * dist / self.lim.return_speed + 3.0) as f64,
                        };
                        return vec![self.mv(self.origin, self.lim.return_speed, self.lim.accel_g)];
                    }
                    return self.start_block(t, pos);
                }
                Vec::new()
            }

            Step::Position { deadline } => {
                if Self::reached(pos, self.origin) {
                    self.start_block(t, pos)
                } else if t >= deadline {
                    self.emergency(t, "move timeout", true)
                } else {
                    Vec::new()
                }
            }

            Step::OutFast { deadline } => {
                if Self::reached(pos, self.leg_target) {
                    self.begin_out_slow(t, pos)
                } else if t >= deadline {
                    self.emergency(t, "move timeout", true)
                } else {
                    Vec::new()
                }
            }

            Step::OutSlow { deadline } => {
                if Self::reached(pos, self.leg_target) {
                    let kind = self.cur.as_ref().map(|b| b.kind);
                    if kind == Some(BlockKind::Hold) {
                        let dwell = self.cur.as_ref().map(|b| b.dwell_s).unwrap_or(0.0);
                        self.step = Step::Dwell {
                            until: t + dwell as f64,
                        };
                        Vec::new()
                    } else {
                        self.begin_back(t, pos)
                    }
                } else if t >= deadline {
                    self.emergency(t, "move timeout", true)
                } else {
                    Vec::new()
                }
            }

            Step::Dwell { until } => {
                let left = (until - t).max(0.0) as f32;
                if left <= 3.0 {
                    self.cue = Some(left);
                }
                if t >= until {
                    self.begin_back(t, pos)
                } else {
                    Vec::new()
                }
            }

            Step::Back { deadline } => {
                if Self::reached(pos, self.leg_target) {
                    let (kind, n) = self
                        .cur
                        .as_ref()
                        .map(|b| (b.kind, b.strokes))
                        .unwrap_or((BlockKind::Hold, 0));
                    if kind == BlockKind::Hold {
                        return self.finish_block(t, pos);
                    }
                    self.stroke_idx += 1;
                    if self.stroke_idx >= n {
                        self.finish_block(t, pos)
                    } else {
                        self.step = Step::Breath {
                            until: t + self.lim.breath_window_s as f64,
                        };
                        Vec::new()
                    }
                } else if t >= deadline {
                    self.emergency(t, "move timeout", true)
                } else {
                    Vec::new()
                }
            }

            Step::Breath { until } => {
                if t >= until {
                    self.begin_out(t, pos)
                } else {
                    Vec::new()
                }
            }

            Step::WarmOut { deadline } => {
                if Self::reached(pos, self.leg_target) {
                    self.begin_warm_back(t, pos)
                } else if t >= deadline {
                    self.emergency(t, "move timeout", true)
                } else {
                    Vec::new()
                }
            }

            Step::WarmBack { deadline } => {
                if Self::reached(pos, self.origin) {
                    if t >= self.warm_until {
                        // Warm-up time is up: park at the origin and wait for the user.
                        self.cur = None;
                        self.rest_total = 0.0;
                        self.step = Step::WarmGate;
                        self.phase = DepthPhase::Warmup;
                        Vec::new()
                    } else {
                        self.begin_warm_out(t, pos)
                    }
                } else if t >= deadline {
                    self.emergency(t, "move timeout", true)
                } else {
                    Vec::new()
                }
            }

            Step::WarmGate => {
                self.phase = DepthPhase::Warmup;
                Vec::new()
            }

            Step::Return { deadline } => {
                if pos <= self.origin + POS_TOL_MM {
                    self.begin_rest(t);
                    Vec::new()
                } else if t >= deadline {
                    self.emergency(t, "move timeout", true)
                } else {
                    Vec::new()
                }
            }

            Step::Emergency { deadline, end } => {
                let reached = pos <= self.origin + POS_TOL_MM;
                if !reached && t < deadline {
                    return Vec::new();
                }
                if !reached {
                    let why = self.emergency.clone().unwrap_or_default();
                    self.end_reason = Some(format!("{why} (retract did not complete)"));
                }
                if end || self.exit_requested || !reached {
                    if self.end_reason.is_none() {
                        self.end_reason = self.emergency.clone();
                    }
                    self.step = Step::Done;
                    self.phase = if self.exit_requested {
                        DepthPhase::Ended
                    } else {
                        DepthPhase::Rating
                    };
                    self.completed = false;
                } else {
                    // First deadman release: abort this block, rest, carry on.
                    self.emergency = None;
                    match self.cur.take() {
                        Some(b) if b.kind == BlockKind::WarmTimed => {
                            // The warm-up stays inside the breathing-safe offset,
                            // so no rest is needed — but it must still be done:
                            // put back what was left of it.
                            let left =
                                (self.warm_until - t).max(self.lim.warmup_extend_s as f64 * 0.25);
                            self.warm_total_s -= (b.dwell_s - left as f32).max(0.0);
                            self.blocks.push_front(Block {
                                dwell_s: left as f32,
                                ..b
                            });
                            self.rest_total = 0.0;
                        }
                        _ => self.rest_total = self.lim.rest_s(self.sub_block_s),
                    }
                    self.begin_rest(t);
                }
                Vec::new()
            }
        }
    }

    /// Copy the machine's view into the shared runtime (not `active`/`driving`).
    pub fn snapshot(&self, t: f64, rt: &mut DepthRuntime) {
        let blk = self.cur.as_ref().or(self.blocks.front());
        rt.phase = self.phase;
        rt.set_index = self.cur.as_ref().map(|b| b.set_idx).unwrap_or(0);
        rt.set_count = blk.map(|b| b.set_count).unwrap_or(0);
        rt.stroke_index = if self.cur.is_some() {
            (self.stroke_idx + 1).min(blk.map(|b| b.strokes).unwrap_or(0))
        } else {
            0
        };
        rt.stroke_count = blk.map(|b| b.strokes).unwrap_or(0);
        rt.target_depth_mm = blk.map(|b| b.depth_mm).unwrap_or(0.0);
        rt.planned_hold_s = blk.filter(|b| b.kind == BlockKind::Hold).map(|b| b.dwell_s);
        rt.depth_beyond_origin_mm = self.depth_beyond;
        rt.submerged_s = self.sub_block_s;
        rt.submerged_cap_s = (self.cap_s > 0.0).then_some(self.cap_s);
        let resting = matches!(self.step, Step::Idle) && t < self.rest_until;
        rt.rest_remaining_s = if resting {
            (self.rest_until - t).max(0.0) as f32
        } else {
            0.0
        };
        rt.rest_total_s = if resting { self.rest_total } else { 0.0 };
        rt.deadman_held = self.held;
        rt.awaiting_deadman = self.awaiting;
        rt.emergency = self.emergency.is_some() || self.end_reason.is_some();
        rt.emergency_reason = self.emergency.clone().or_else(|| self.end_reason.clone());
        rt.early_release = self.early_releases > 0;
        rt.cue_soon_s = self.cue;
        let gate = matches!(self.step, Step::WarmGate);
        let warming = matches!(self.step, Step::WarmOut { .. } | Step::WarmBack { .. })
            || self
                .cur
                .as_ref()
                .is_some_and(|b| b.kind == BlockKind::WarmTimed);
        rt.warmup_remaining_s = if warming {
            Some((self.warm_until - t).max(0.0) as f32)
        } else if gate {
            Some(0.0)
        } else {
            None
        };
        rt.awaiting_warmup_confirm = gate;
        rt.warmup_can_extend = gate && self.can_extend_warmup();
    }
}

// ─────────────────────────────── tokio task ───────────────────────────────

struct Session {
    machine: Machine,
    targets: Targets,
    t0: Instant,
    last_seq: u64,
    last_seq_at: Instant,
}

pub struct DepthTask {
    state: Arc<RwLock<AppState>>,
    cmd_tx: mpsc::Sender<ActuatorCommand>,
    lim: Limits,
}

impl DepthTask {
    pub fn new(
        state: Arc<RwLock<AppState>>,
        cmd_tx: mpsc::Sender<ActuatorCommand>,
        cfg: &Config,
    ) -> Self {
        DepthTask {
            state,
            cmd_tx,
            lim: Limits::from_config(cfg),
        }
    }

    pub fn limits(&self) -> &Limits {
        &self.lim
    }

    async fn exec(&self, actions: Vec<Action>) {
        for a in actions {
            let cmd = match a {
                Action::Stop => ActuatorCommand::Stop,
                Action::MoveTo {
                    pos_mm,
                    vel_mm_s,
                    accel_g,
                    profile,
                } => ActuatorCommand::MoveTo {
                    pos_mm,
                    vel_mm_s,
                    accel_g,
                    profile,
                    soften: false,
                },
            };
            let _ = self.cmd_tx.send(cmd).await;
        }
    }

    fn persist(&self, p: &Progress) {
        if let Err(e) = p.save(&self.lim.progress_path) {
            warn!(error = %e, "depth trainer: failed to persist progress; kept in memory only");
        }
    }

    pub async fn run(self, mut ctrl_rx: mpsc::Receiver<DepthControl>) {
        info!("depth trainer task running");
        let mut ticker = tokio::time::interval(TICK);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
        let mut session: Option<Session> = None;

        loop {
            tokio::select! {
                ctrl = ctrl_rx.recv() => match ctrl {
                    None => {
                        // Shutting down: retract best-effort.
                        if let Some(s) = session.as_mut() {
                            let t = s.t0.elapsed().as_secs_f64();
                            let acts = s.machine.stop(t);
                            self.exec(acts).await;
                        }
                        break;
                    }
                    Some(c) => self.handle(c, &mut session).await,
                },
                _ = ticker.tick() => {
                    if session.is_some() {
                        self.tick(&mut session).await;
                    }
                }
            }
        }
        info!("depth trainer task stopped");
    }

    async fn handle(&self, c: DepthControl, session: &mut Option<Session>) {
        match c {
            DepthControl::Start => {
                if session.is_some() {
                    return;
                }
                let now = now_ms();
                let (res, mode_ok) = {
                    let st = self.state.read().await;
                    let ctx = StartCtx::from_state(&st);
                    (st.depth_progress.can_start(&self.lim, &ctx, now), ctx.idle)
                };
                let (targets, cap, origin, blocks) = match res {
                    Ok(x) => x,
                    Err(reason) => {
                        warn!(%reason, mode_ok, "depth trainer: start refused");
                        return;
                    }
                };
                let offset = self.state.read().await.depth_progress.breath_offset_mm;
                let _ = self.cmd_tx.send(ActuatorCommand::ServoOn(true)).await;
                tokio::time::sleep(SERVO_SETTLE).await;
                let machine = Machine::new(self.lim.clone(), origin, offset, cap, blocks, 0.0);
                {
                    let mut st = self.state.write().await;
                    st.depth = DepthRuntime {
                        active: true,
                        driving: true,
                        ..Default::default()
                    };
                    st.set_mode(AppMode::DepthTrainer);
                }
                let (seq, at) = (self.state.read().await.status_seq, Instant::now());
                *session = Some(Session {
                    machine,
                    targets,
                    t0: Instant::now(),
                    last_seq: seq,
                    last_seq_at: at,
                });
                info!(origin, cap, offset, ?targets, "depth trainer: armed");
            }

            DepthControl::Stop => {
                let Some(s) = session.as_mut() else { return };
                let t = s.t0.elapsed().as_secs_f64();
                let acts = s.machine.stop(t);
                self.exec(acts).await;
                if s.machine.is_done() {
                    self.finish(session, None).await;
                }
            }

            DepthControl::Settings {
                breath_hold_s,
                breath_offset_mm,
            } => {
                if session.is_some() {
                    warn!("depth trainer: settings ignored while a session is active");
                    return;
                }
                let snap = {
                    let mut st = self.state.write().await;
                    match st
                        .depth_progress
                        .set_settings(breath_hold_s, breath_offset_mm)
                    {
                        Ok(()) => Some(st.depth_progress.clone()),
                        Err(e) => {
                            warn!(error = %e, "depth trainer: settings rejected");
                            None
                        }
                    }
                };
                if let Some(p) = snap {
                    self.persist(&p);
                }
            }

            DepthControl::Rate(r) => {
                let ready = session
                    .as_ref()
                    .map(|s| s.machine.is_done() && !s.machine.exit_requested())
                    .unwrap_or(false);
                if ready {
                    self.finish(session, Some(r)).await;
                }
            }

            DepthControl::WarmupContinue => {
                if let Some(s) = session.as_mut() {
                    s.machine.warmup_continue();
                }
            }

            DepthControl::WarmupExtend => {
                if let Some(s) = session.as_mut() {
                    s.machine.warmup_extend();
                }
            }

            DepthControl::StepBack => {
                if session.is_some() {
                    return;
                }
                let snap = {
                    let mut st = self.state.write().await;
                    if st.mode != AppMode::Idle {
                        return;
                    }
                    st.depth_progress.step_back(&self.lim, now_ms());
                    st.depth_progress.clone()
                };
                self.persist(&snap);
            }
        }
    }

    async fn tick(&self, session: &mut Option<Session>) {
        let Some(s) = session.as_mut() else { return };
        let mode_changed;
        let sample = {
            let st = self.state.read().await;
            if st.status_seq != s.last_seq {
                s.last_seq = st.status_seq;
                s.last_seq_at = Instant::now();
            }
            let stale = s.last_seq_at.elapsed() > self.lim.deadman_timeout;
            let fault = if !st.actuator_connected {
                Some("actuator link lost")
            } else if st.calibrating {
                Some("calibration started")
            } else if st.emergency_stop {
                Some("emergency stop")
            } else if st.alarm_major {
                Some("controller alarm")
            } else if stale {
                Some("status data stale")
            } else {
                None
            };
            mode_changed = st.mode != AppMode::DepthTrainer;
            Sample {
                position_mm: st.position_mm,
                held: st.hand_switch && fault.is_none(),
                fault,
            }
        };
        let t = s.t0.elapsed().as_secs_f64();
        let mut actions = s.machine.tick(t, &sample);
        if mode_changed && !s.machine.exit_requested() {
            // Something else took over the mode (another program, StopAll,
            // transport drop): retract and leave rather than fight over the rod.
            actions.extend(s.machine.stop(t));
        }
        if !actions.is_empty() {
            self.exec(actions).await;
        }
        {
            let mut st = self.state.write().await;
            s.machine.snapshot(t, &mut st.depth);
        }
        if s.machine.is_done() {
            if s.machine.exit_requested() {
                self.finish(session, None).await;
            } else {
                // Rod is back at the origin and holding; wait for the rating.
                let mut st = self.state.write().await;
                st.depth.driving = false;
            }
        }
    }

    /// End the session: record progress, hand the mode back to Idle. The servo
    /// is deliberately left energised — the rod is at the origin with no brake.
    async fn finish(&self, session: &mut Option<Session>, rating: Option<DepthRating>) {
        let Some(s) = session.take() else { return };
        let outcome = s.machine.outcome();
        let now = now_ms();
        let snap = {
            let mut st = self.state.write().await;
            match rating {
                Some(r) => st
                    .depth_progress
                    .apply_rating(r, &s.targets, &outcome, &self.lim, now),
                None => st.depth_progress.record_unrated(&s.targets, &outcome, now),
            }
            st.depth.active = false;
            st.depth.driving = false;
            st.depth.phase = DepthPhase::Ended;
            if st.mode == AppMode::DepthTrainer {
                st.set_mode(AppMode::Idle);
            }
            st.depth_progress.clone()
        };
        self.persist(&snap);
        info!(?rating, early = outcome.early_releases, reason = ?outcome.emergency, "depth trainer: session ended");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    impl Machine {
        fn snapshot_phase(&self) -> DepthPhase {
            self.phase
        }
    }

    fn lim() -> Limits {
        Limits::from_parts(&DepthCfg::default(), 300.0, 300.0, MotionProfile::SCurve)
    }

    fn targets() -> Targets {
        Targets {
            osc_depth_mm: 10.0,
            hold_depth_mm: 5.0,
            hold_s: 5.0,
        }
    }

    // ── limits & budget ──

    #[test]
    fn hostile_config_cannot_loosen_safety_bounds() {
        let c = DepthCfg {
            breath_fraction: 5.0,
            max_hold_s: 999.0,
            rest_factor: 0.0,
            min_rest_s: 0.0,
            breath_window_s: 0.0,
            step_mm: 99.0,
            advance_cooldown_h: 0.0,
            retract_speed_mm_s: 0.0,
            speed_mm_s: 99.0,
            deadman_timeout_ms: 60_000,
            ..DepthCfg::default()
        };
        let l = Limits::from_parts(&c, 300.0, 300.0, MotionProfile::SCurve);
        assert!(l.breath_fraction <= ABS_MAX_FRACTION);
        assert!(l.max_hold_s <= ABS_MAX_HOLD_S);
        assert!(l.rest_factor >= MIN_REST_FACTOR && l.min_rest_s >= MIN_REST_S);
        assert!(l.breath_window_s >= MIN_BREATH_WINDOW_S);
        assert!(l.step_mm <= MAX_STEP_MM);
        assert!(l.cooldown_ms >= (MIN_COOLDOWN_H as f64 * 3_600_000.0) as u64);
        assert!(
            l.retract_speed >= l.speed * 2.0,
            "retract must stay faster than advance"
        );
        assert!(l.deadman_timeout <= Duration::from_millis(300));
        // NaN can't slip through either.
        let c = DepthCfg {
            breath_fraction: f32::NAN,
            max_hold_s: f32::NAN,
            ..DepthCfg::default()
        };
        let l = Limits::from_parts(&c, 300.0, 300.0, MotionProfile::SCurve);
        assert_eq!(l.cap_s(1000.0), ABS_MAX_HOLD_S);
    }

    #[test]
    fn cap_is_fraction_of_breath_hold_and_never_above_absolute() {
        let l = lim();
        assert!((l.cap_s(30.0) - 18.0).abs() < 1e-3); // 0.6 × 30
        assert_eq!(l.cap_s(60.0), ABS_MAX_HOLD_S); // 36 → clamped to 20
        assert_eq!(l.cap_s(10_000.0), ABS_MAX_HOLD_S);
    }

    #[test]
    fn rest_is_three_times_submerged_and_at_least_twenty_seconds() {
        let l = lim();
        assert_eq!(l.rest_s(2.0), 20.0);
        assert_eq!(l.rest_s(10.0), 30.0);
        assert_eq!(l.rest_s(0.0), 20.0);
    }

    #[test]
    fn transit_counts_against_the_hold_budget_and_unreachable_holds_are_refused() {
        let l = lim();
        let cap = l.cap_s(30.0); // 18 s
        let shallow = l.dwell_budget_s(cap, 5.0, 0.0);
        let deep = l.dwell_budget_s(cap, 40.0, 0.0);
        assert!(shallow > deep, "deeper hold must have less dwell");
        assert!(shallow < cap - 1.0, "transit and margin must be deducted");
        let t = Targets {
            osc_depth_mm: 10.0,
            hold_depth_mm: 60.0,
            hold_s: 5.0,
        };
        assert!(matches!(
            plan_session(&l, &t, cap, 0.0),
            Err(PlanError::HoldUnreachable { .. })
        ));
        assert_eq!(
            plan_session(&l, &targets(), 3.0, 0.0).unwrap_err(),
            PlanError::CapTooShort(3.0)
        );
    }

    #[test]
    fn set_length_is_limited_by_cumulative_submerged_time() {
        let l = lim();
        let cap = l.cap_s(30.0);
        assert_eq!(
            l.strokes_in_set(cap, 5.0, 0.0),
            l.strokes_per_set.min(l.strokes_in_set(cap, 5.0, 0.0))
        );
        let deep = l.strokes_in_set(cap, 40.0, 0.0);
        assert!(deep < l.strokes_per_set);
        // a breathing-safe offset removes the shallow part from the budget
        assert!(l.strokes_in_set(cap, 40.0, 30.0) > deep);
    }

    #[test]
    fn planned_hold_never_exceeds_budget_or_cap() {
        let l = lim();
        for breath in [10.0_f32, 20.0, 30.0, 60.0, 180.0] {
            let cap = l.cap_s(breath);
            let t = Targets {
                osc_depth_mm: 10.0,
                hold_depth_mm: 5.0,
                hold_s: 99.0,
            };
            if let Ok((blocks, eff)) = plan_session(&l, &t, cap, 0.0) {
                for b in blocks.iter().filter(|b| b.kind == BlockKind::Hold) {
                    assert!(b.dwell_s <= cap, "dwell {} > cap {}", b.dwell_s, cap);
                    assert!(b.dwell_s <= l.dwell_budget_s(cap, b.depth_mm, 0.0).max(MIN_DWELL_S));
                }
                assert!(eff.hold_s <= cap);
            }
        }
    }

    // ── progression ──

    const DAY: u64 = 24 * 3_600_000;

    fn clean() -> Outcome {
        Outcome {
            early_releases: 0,
            emergency: None,
            completed: true,
        }
    }

    fn prog() -> Progress {
        let mut p = Progress::default();
        p.set_settings(30.0, 0.0).unwrap();
        p
    }

    #[test]
    fn first_session_uses_conservative_start_values() {
        let l = lim();
        let t = Progress::default().next_targets(&l, 0, 200.0, 18.0, 0.0);
        assert_eq!(
            t,
            Targets {
                osc_depth_mm: 10.0,
                hold_depth_mm: 5.0,
                hold_s: 5.0
            }
        );
    }

    #[test]
    fn comfortable_advances_hold_time_first_then_depth_and_respects_cooldown() {
        let l = lim();
        let mut p = prog();
        let t0 = Progress::default().next_targets(&l, 0, 200.0, 18.0, 0.0);
        p.apply_rating(DepthRating::Comfortable, &t0, &clean(), &l, 1000);
        assert_eq!(p.osc_comfortable_mm, Some(10.0));
        // inside the cooldown: no increase
        let same = p.next_targets(&l, 1000 + DAY - 1, 200.0, 18.0, 0.0);
        assert_eq!(same, t0);
        // after the cooldown: +1 s hold, +2 mm oscillation
        let next = p.next_targets(&l, 1000 + DAY, 200.0, 18.0, 0.0);
        assert_eq!(next.osc_depth_mm, 12.0);
        assert_eq!(next.hold_depth_mm, 5.0);
        assert_eq!(next.hold_s, 6.0);
        // when the dwell budget is exhausted the hold depth steps and time resets
        let budget = l.dwell_budget_s(18.0, 5.0, 0.0);
        p.hold_comfortable_s = Some(budget.floor());
        let n2 = p.next_targets(&l, 1000 + DAY, 200.0, 18.0, 0.0);
        assert_eq!(n2.hold_depth_mm, 7.0);
        assert_eq!(n2.hold_s, l.start_hold_s);
    }

    #[test]
    fn early_release_or_non_comfortable_rating_never_advances() {
        let l = lim();
        let t = targets();
        let mut p = prog();
        let early = Outcome {
            early_releases: 1,
            emergency: None,
            completed: true,
        };
        p.apply_rating(DepthRating::Comfortable, &t, &early, &l, 5);
        assert_eq!(p.osc_comfortable_mm, None);
        assert_eq!(p.last_progress_ms, None);
        p.apply_rating(DepthRating::Okay, &t, &clean(), &l, 5);
        assert_eq!(p.osc_comfortable_mm, None);
        let aborted = Outcome {
            early_releases: 0,
            emergency: Some("alarm".into()),
            completed: false,
        };
        p.apply_rating(DepthRating::Comfortable, &t, &aborted, &l, 5);
        assert_eq!(p.osc_comfortable_mm, None);
    }

    #[test]
    fn too_much_steps_back_and_restarts_the_cooldown() {
        let l = lim();
        let mut p = prog();
        let t = Targets {
            osc_depth_mm: 14.0,
            hold_depth_mm: 7.0,
            hold_s: 6.0,
        };
        p.osc_comfortable_mm = Some(14.0);
        p.hold_comfortable_mm = Some(7.0);
        p.hold_comfortable_s = Some(6.0);
        p.apply_rating(DepthRating::TooMuch, &t, &clean(), &l, 777);
        assert_eq!(p.osc_comfortable_mm, Some(12.0));
        assert_eq!(p.hold_comfortable_s, Some(5.0));
        assert_eq!(p.last_progress_ms, Some(777));
        // an advance attempt rated too much leaves the stored values alone
        let mut q = prog();
        q.osc_comfortable_mm = Some(10.0);
        q.hold_comfortable_mm = Some(5.0);
        q.hold_comfortable_s = Some(5.0);
        let attempted = Targets {
            osc_depth_mm: 12.0,
            hold_depth_mm: 5.0,
            hold_s: 6.0,
        };
        q.apply_rating(DepthRating::TooMuch, &attempted, &clean(), &l, 9);
        assert_eq!(q.osc_comfortable_mm, Some(10.0));
        assert_eq!(q.last_progress_ms, Some(9));
    }

    #[test]
    fn step_back_is_suggested_after_two_early_release_sessions() {
        let l = lim();
        let mut p = prog();
        let early = Outcome {
            early_releases: 1,
            emergency: None,
            completed: true,
        };
        p.apply_rating(DepthRating::Okay, &targets(), &early, &l, 1);
        assert!(!p.suggest_step_back());
        p.apply_rating(DepthRating::Okay, &targets(), &early, &l, 2);
        assert!(p.suggest_step_back());
        p.step_back(&l, 3);
        assert!(!p.suggest_step_back());
        assert_eq!(p.osc_comfortable_mm, Some(8.0));
    }

    #[test]
    fn targets_are_clamped_to_the_depth_ceiling() {
        let l = lim();
        let mut p = prog();
        p.osc_comfortable_mm = Some(40.0);
        p.hold_comfortable_mm = Some(40.0);
        p.hold_comfortable_s = Some(5.0);
        let t = p.next_targets(&l, DAY * 10, 25.0, 18.0, 0.0);
        assert!(t.osc_depth_mm <= 25.0 && t.hold_depth_mm <= 25.0);
    }

    #[test]
    fn progress_round_trips_and_corrupt_or_hostile_files_fall_back_conservatively() {
        let dir =
            std::env::temp_dir().join(format!("rod-depth-{}-{}", std::process::id(), now_ms()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("p.json");
        let mut p = prog();
        p.osc_comfortable_mm = Some(14.0);
        p.hold_comfortable_mm = Some(6.0);
        p.hold_comfortable_s = Some(7.0);
        p.last_progress_ms = Some(42);
        p.save(&path).unwrap();
        assert_eq!(Progress::load(&path), p);

        std::fs::write(&path, b"{ not json").unwrap();
        assert_eq!(Progress::load(&path), Progress::default());

        // out-of-range values are dropped, never kept or enlarged
        std::fs::write(
            &path,
            br#"{"breath_hold_s": 99999, "osc_comfortable_mm": 5000, "hold_comfortable_mm": 5, "hold_comfortable_s": 500, "breath_offset_mm": -3}"#,
        )
        .unwrap();
        let q = Progress::load(&path);
        assert_eq!(q.breath_hold_s, None);
        assert_eq!(q.osc_comfortable_mm, None);
        assert_eq!(q.hold_comfortable_mm, None);
        assert_eq!(q.breath_offset_mm, 0.0);
        assert_eq!(
            Progress::load(&dir.join("missing.json")),
            Progress::default()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn can_start_requires_calibration_breath_hold_and_a_healthy_idle_machine() {
        let l = lim();
        let ok_ctx = StartCtx {
            idle: true,
            connected: true,
            faulted: false,
            calibrating: false,
            origin: Some(50.0),
            max_depth_mm: 0.0,
        };
        assert!(prog().can_start(&l, &ok_ctx, 0).is_ok());
        let no_cal = StartCtx {
            origin: None,
            ..ok_ctx
        };
        assert!(prog()
            .can_start(&l, &no_cal, 0)
            .unwrap_err()
            .contains("calibrat"));
        assert!(Progress::default()
            .can_start(&l, &ok_ctx, 0)
            .unwrap_err()
            .contains("breath-hold"));
        assert!(prog()
            .can_start(
                &l,
                &StartCtx {
                    idle: false,
                    ..ok_ctx
                },
                0
            )
            .is_err());
        assert!(prog()
            .can_start(
                &l,
                &StartCtx {
                    connected: false,
                    ..ok_ctx
                },
                0
            )
            .is_err());
        assert!(prog()
            .can_start(
                &l,
                &StartCtx {
                    faulted: true,
                    ..ok_ctx
                },
                0
            )
            .is_err());
        // max-depth ceiling leaving no room below the origin
        assert!(prog()
            .can_start(
                &l,
                &StartCtx {
                    max_depth_mm: 51.0,
                    ..ok_ctx
                },
                0
            )
            .is_err());
    }

    // ── state machine ──

    /// Tiny kinematic sim: the rod chases the last commanded target at its speed.
    struct Sim {
        pos: f32,
        target: f32,
        vel: f32,
        moves: Vec<(f64, f32)>,
        stops: u32,
    }

    impl Sim {
        fn new(pos: f32) -> Self {
            Sim {
                pos,
                target: pos,
                vel: 0.0,
                moves: vec![],
                stops: 0,
            }
        }
        fn apply(&mut self, t: f64, acts: &[Action]) {
            for a in acts {
                match a {
                    Action::Stop => {
                        self.target = self.pos;
                        self.stops += 1;
                    }
                    Action::MoveTo {
                        pos_mm, vel_mm_s, ..
                    } => {
                        self.target = *pos_mm;
                        self.vel = *vel_mm_s;
                        self.moves.push((t, *pos_mm));
                    }
                }
            }
        }
        fn step(&mut self, dt: f32) {
            let d = self.target - self.pos;
            let m = (self.vel * dt).min(d.abs());
            self.pos += m * d.signum();
        }
    }

    const ORIGIN: f32 = 50.0;
    const DT: f64 = 0.05;

    fn machine(offset: f32, breath: f32) -> (Machine, f32) {
        let l = lim();
        let cap = l.cap_s(breath);
        let (blocks, _) = plan_session(&l, &targets(), cap, offset).unwrap();
        (Machine::new(l, ORIGIN, offset, cap, blocks, 0.0), cap)
    }

    /// Run until `pred` or timeout, switch held per `held(t)`.
    fn run(
        m: &mut Machine,
        sim: &mut Sim,
        mut t: f64,
        until: f64,
        held: impl Fn(f64) -> bool,
        mut pred: impl FnMut(&Machine, &Sim) -> bool,
    ) -> f64 {
        while t < until {
            t += DT;
            sim.step(DT as f32);
            m.warmup_continue(); // no-op unless waiting at the warm-up gate
            let acts = m.tick(
                t,
                &Sample {
                    position_mm: sim.pos,
                    held: held(t),
                    fault: None,
                },
            );
            sim.apply(t, &acts);
            if pred(m, sim) {
                break;
            }
        }
        t
    }

    #[test]
    fn nothing_moves_until_the_switch_is_held_and_the_countdown_has_run() {
        let (mut m, _) = machine(0.0, 30.0);
        let mut sim = Sim::new(ORIGIN);
        let t = run(&mut m, &mut sim, 0.0, 10.0, |_| false, |_, _| false);
        assert!(sim.moves.is_empty(), "no motion without the deadman");
        assert!(t >= 10.0);
        // held: countdown (3 s) must elapse before the first move
        let t1 = run(&mut m, &mut sim, t, t + 2.5, |_| true, |_, _| false);
        assert!(sim.moves.is_empty(), "still counting down");
        run(&mut m, &mut sim, t1, t1 + 1.0, |_| true, |_, _| false);
        assert!(!sim.moves.is_empty(), "moves after the countdown");
    }

    #[test]
    fn releasing_the_switch_mid_stroke_stops_then_retracts_to_origin_at_retract_speed() {
        let (mut m, _) = machine(0.0, 30.0);
        let l = lim();
        let mut sim = Sim::new(ORIGIN);
        // go until the rod is clearly out beyond the origin
        let t = run(
            &mut m,
            &mut sim,
            0.0,
            60.0,
            |_| true,
            |_, s| s.pos > ORIGIN + 3.0,
        );
        assert!(sim.pos > ORIGIN + 3.0);
        let before = sim.moves.len();
        let acts = m.tick(
            t + DT,
            &Sample {
                position_mm: sim.pos,
                held: false,
                fault: None,
            },
        );
        assert_eq!(acts[0], Action::Stop, "first action is a decel stop");
        match &acts[1] {
            Action::MoveTo {
                pos_mm, vel_mm_s, ..
            } => {
                assert_eq!(*pos_mm, ORIGIN, "retract targets the work-piece origin");
                assert_eq!(*vel_mm_s, l.retract_speed);
                assert!(*vel_mm_s > l.speed, "retract is faster than advance");
            }
            a => panic!("expected MoveTo, got {a:?}"),
        }
        assert_eq!(acts.len(), 2);
        assert_eq!(m.snapshot_phase(), DepthPhase::Retracting);
        sim.apply(t, &acts);
        // the rod gets back and the machine rests; it never advances without the switch again
        run(&mut m, &mut sim, t, t + 60.0, |_| false, |_, _| false);
        assert!((sim.pos - ORIGIN).abs() <= POS_TOL_MM);
        assert_eq!(
            sim.moves.len(),
            before + 1,
            "only the retract move after release"
        );
    }

    #[test]
    fn releasing_during_the_normal_return_speeds_it_up_without_counting_as_early_release() {
        // offset 4 mm: strokes turn around above the origin, so a block ends with a Return
        let (mut m, _) = machine(4.0, 30.0);
        let l = lim();
        let mut sim = Sim::new(ORIGIN);
        // run until the first block's return begins
        let t = run(
            &mut m,
            &mut sim,
            0.0,
            200.0,
            |_| true,
            |m, _| matches!(m.step, Step::Return { .. }),
        );
        assert!(
            matches!(m.step, Step::Return { .. }),
            "reached the end-of-block return"
        );
        let acts = m.tick(
            t + DT,
            &Sample {
                position_mm: sim.pos.max(ORIGIN + 2.0),
                held: false,
                fault: None,
            },
        );
        assert_eq!(acts.len(), 1);
        assert!(
            matches!(acts[0], Action::MoveTo { pos_mm, vel_mm_s, .. } if pos_mm == ORIGIN && vel_mm_s == l.retract_speed)
        );
        assert_eq!(m.outcome().early_releases, 0);
        // only once
        let again = m.tick(
            t + 2.0 * DT,
            &Sample {
                position_mm: sim.pos.max(ORIGIN + 2.0),
                held: false,
                fault: None,
            },
        );
        assert!(again.is_empty());
    }

    // ── timed warm-up ──

    fn tick_n(m: &mut Machine, sim: &mut Sim, t: &mut f64, secs: f64, held: bool) {
        let end = *t + secs;
        while *t < end {
            *t += DT;
            sim.step(DT as f32);
            let acts = m.tick(
                *t,
                &Sample {
                    position_mm: sim.pos,
                    held,
                    fault: None,
                },
            );
            sim.apply(*t, &acts);
        }
    }

    #[test]
    fn timed_warmup_is_planned_only_when_the_offset_leaves_room() {
        let l = lim();
        let cap = l.cap_s(30.0);
        let (with_room, _) = plan_session(&l, &targets(), cap, 4.0).unwrap();
        assert_eq!(with_room[0].kind, BlockKind::WarmTimed);
        assert_eq!(with_room[0].dwell_s, l.warmup_s);
        assert!(
            with_room[0].depth_mm <= 4.0,
            "warm-up stays within the offset"
        );
        let (no_room, _) = plan_session(&l, &targets(), cap, 0.0).unwrap();
        assert_eq!(
            no_room[0].kind,
            BlockKind::Warmup,
            "falls back to the short strokes"
        );
        let (tiny, _) = plan_session(&l, &targets(), cap, 1.0).unwrap();
        assert_eq!(tiny[0].kind, BlockKind::Warmup);
        // never deeper than half the working depth even with a big offset
        let (big, _) = plan_session(&l, &targets(), cap, 30.0).unwrap();
        assert_eq!(big[0].kind, BlockKind::WarmTimed);
        assert!(big[0].depth_mm <= 5.0 + 1e-3);
    }

    #[test]
    fn timed_warmup_stays_inside_the_offset_then_waits_at_the_gate() {
        let (mut m, _) = machine(4.0, 30.0);
        let l = lim();
        let mut sim = Sim::new(ORIGIN);
        let mut t = 0.0;
        let mut max_pos = ORIGIN;
        let mut max_sub = 0.0f32;
        // countdown + full warm-up + a little slack
        let end = 3.0 + l.warmup_s as f64 + 40.0;
        while t < end && !matches!(m.step, Step::WarmGate) {
            tick_n(&mut m, &mut sim, &mut t, DT, true);
            max_pos = max_pos.max(sim.pos);
            max_sub = max_sub.max(m.sub_block_s);
        }
        assert!(
            matches!(m.step, Step::WarmGate),
            "warm-up should end at the gate"
        );
        assert!(t >= l.warmup_s as f64, "ran the whole warm-up ({t} s)");
        assert!(
            max_pos <= ORIGIN + 4.0 + POS_TOL_MM,
            "stayed inside the offset: {max_pos}"
        );
        assert_eq!(max_sub, 0.0, "never submerged → no breath budget used");
        assert!(
            (sim.pos - ORIGIN).abs() <= POS_TOL_MM,
            "parked at the origin"
        );
        // waiting at the gate: nothing moves even with the switch held
        let moves = sim.moves.len();
        tick_n(&mut m, &mut sim, &mut t, 30.0, true);
        assert_eq!(sim.moves.len(), moves, "no motion until the user confirms");
        let mut rt = DepthRuntime::default();
        m.snapshot(t, &mut rt);
        assert!(rt.awaiting_warmup_confirm && rt.warmup_can_extend);
        assert_eq!(rt.warmup_remaining_s, Some(0.0));
        assert_eq!(rt.phase, DepthPhase::Warmup);
    }

    #[test]
    fn continue_starts_the_main_blocks_after_only_the_normal_countdown() {
        let (mut m, _) = machine(4.0, 30.0);
        let l = lim();
        let mut sim = Sim::new(ORIGIN);
        let mut t = 0.0;
        while !matches!(m.step, Step::WarmGate) {
            tick_n(&mut m, &mut sim, &mut t, DT, true);
        }
        assert!(m.warmup_continue());
        assert!(!m.warmup_continue(), "second continue is a no-op");
        let moves = sim.moves.len();
        let t0 = t;
        while sim.moves.len() == moves && t < t0 + 20.0 {
            tick_n(&mut m, &mut sim, &mut t, DT, true);
        }
        let waited = t - t0;
        assert!(
            waited >= l.ready_countdown_s as f64 - 0.2,
            "countdown first ({waited})"
        );
        assert!(
            waited < 6.0,
            "no rest after a warm-up that never submerged ({waited})"
        );
        assert_eq!(m.snapshot_phase(), DepthPhase::Oscillation);
    }

    #[test]
    fn extend_adds_warmup_time_needs_the_switch_and_is_capped() {
        let (mut m, _) = machine(4.0, 30.0);
        let l = lim();
        let mut sim = Sim::new(ORIGIN);
        let mut t = 0.0;
        let mut extensions = 0;
        loop {
            while !matches!(m.step, Step::WarmGate) {
                tick_n(&mut m, &mut sim, &mut t, DT, true);
            }
            if !m.warmup_extend() {
                break;
            }
            extensions += 1;
            // not held → nothing happens (needs the switch and countdown again)
            let moves = sim.moves.len();
            tick_n(&mut m, &mut sim, &mut t, 5.0, false);
            assert_eq!(sim.moves.len(), moves, "extension still needs the deadman");
            let started = t;
            while !matches!(m.step, Step::WarmGate) {
                tick_n(&mut m, &mut sim, &mut t, DT, true);
            }
            assert!(
                t - started >= l.warmup_extend_s as f64,
                "extension ran its time"
            );
            assert!(extensions < 50);
        }
        let total = l.warmup_s + extensions as f32 * l.warmup_extend_s;
        assert!(
            total <= l.warmup_max_s + 1e-3,
            "total {total} over the ceiling"
        );
        assert!(
            total + l.warmup_extend_s > l.warmup_max_s,
            "stopped only at the ceiling"
        );
        let mut rt = DepthRuntime::default();
        m.snapshot(t, &mut rt);
        assert!(!rt.warmup_can_extend);
    }

    #[test]
    fn releasing_during_the_warmup_retracts_and_the_warmup_is_not_skipped() {
        let (mut m, _) = machine(4.0, 30.0);
        let mut sim = Sim::new(ORIGIN);
        let mut t = 0.0;
        // start the warm-up and release mid-stroke
        while !matches!(m.step, Step::WarmOut { .. } | Step::WarmBack { .. }) {
            tick_n(&mut m, &mut sim, &mut t, DT, true);
        }
        tick_n(&mut m, &mut sim, &mut t, 1.0, false);
        assert!(m.outcome().early_releases == 1);
        assert!((sim.pos - ORIGIN).abs() <= 4.5);
        // re-hold: the warm-up resumes (no rest needed) and ends at the gate, not in the main blocks
        let mut got_gate = false;
        let end = t + 200.0;
        while t < end {
            tick_n(&mut m, &mut sim, &mut t, DT, true);
            if matches!(m.step, Step::WarmGate) {
                got_gate = true;
                break;
            }
            assert!(
                m.snapshot_phase() != DepthPhase::Oscillation,
                "must not skip the warm-up"
            );
        }
        assert!(got_gate, "warm-up resumed and reached the gate");
    }

    #[test]
    fn stop_at_the_warmup_gate_exits_immediately() {
        let (mut m, _) = machine(4.0, 30.0);
        let mut sim = Sim::new(ORIGIN);
        let mut t = 0.0;
        while !matches!(m.step, Step::WarmGate) {
            tick_n(&mut m, &mut sim, &mut t, DT, true);
        }
        assert!(m.stop(t).is_empty());
        assert!(m.is_done());
    }

    #[test]
    fn second_release_ends_the_session_and_the_outcome_blocks_progress() {
        let (mut m, _) = machine(0.0, 30.0);
        let mut sim = Sim::new(ORIGIN);
        let mut t = 0.0;
        for _ in 0..2 {
            t = run(
                &mut m,
                &mut sim,
                t,
                t + 120.0,
                |_| true,
                |_, s| s.pos > ORIGIN + 3.0,
            );
            let acts = m.tick(
                t + DT,
                &Sample {
                    position_mm: sim.pos,
                    held: false,
                    fault: None,
                },
            );
            sim.apply(t, &acts);
            t = run(
                &mut m,
                &mut sim,
                t,
                t + 120.0,
                |_| false,
                |m, _| m.is_done() || m.step == Step::Idle,
            );
            // let the rest elapse so the next round can start
            t = run(
                &mut m,
                &mut sim,
                t,
                t + 200.0,
                |_| false,
                |m, _| m.is_done() || (m.step == Step::Idle && m.awaiting),
            );
        }
        assert!(m.is_done());
        let o = m.outcome();
        assert_eq!(o.early_releases, 2);
        assert!(!o.completed);
    }

    #[test]
    fn faults_end_the_session_with_a_retract() {
        for reason in [
            "actuator link lost",
            "emergency stop",
            "controller alarm",
            "status data stale",
        ] {
            let (mut m, _) = machine(0.0, 30.0);
            let mut sim = Sim::new(ORIGIN);
            let t = run(
                &mut m,
                &mut sim,
                0.0,
                60.0,
                |_| true,
                |_, s| s.pos > ORIGIN + 3.0,
            );
            let acts = m.tick(
                t + DT,
                &Sample {
                    position_mm: sim.pos,
                    held: false,
                    fault: Some(reason),
                },
            );
            assert_eq!(acts[0], Action::Stop);
            assert!(matches!(acts[1], Action::MoveTo { pos_mm, .. } if pos_mm == ORIGIN));
            sim.apply(t, &acts);
            run(&mut m, &mut sim, t, t + 60.0, |_| false, |m, _| m.is_done());
            assert!(m.is_done(), "{reason}");
            assert_eq!(m.outcome().emergency.as_deref(), Some(reason));
        }
    }

    #[test]
    fn stop_command_retracts_when_out_and_exits_immediately_when_at_rest() {
        let (mut m, _) = machine(0.0, 30.0);
        let mut sim = Sim::new(ORIGIN);
        let t = run(
            &mut m,
            &mut sim,
            0.0,
            60.0,
            |_| true,
            |_, s| s.pos > ORIGIN + 3.0,
        );
        let acts = m.stop(t);
        assert_eq!(acts[0], Action::Stop);
        assert!(matches!(acts[1], Action::MoveTo { pos_mm, .. } if pos_mm == ORIGIN));
        sim.apply(t, &acts);
        run(&mut m, &mut sim, t, t + 60.0, |_| true, |m, _| m.is_done());
        assert!(m.is_done() && m.exit_requested());

        let (mut idle, _) = machine(0.0, 30.0);
        assert!(idle.stop(0.0).is_empty());
        assert!(idle.is_done());
    }

    #[test]
    fn watchdog_retracts_if_a_move_runs_far_slower_than_planned() {
        let (mut m, cap) = machine(0.0, 30.0);
        let mut sim = Sim::new(ORIGIN);
        // rod crawls: a stuck/slow actuator must not keep the user submerged
        let mut t = 0.0;
        let mut tripped = false;
        for _ in 0..4000 {
            t += DT;
            if sim.vel > 0.0 {
                sim.vel = 0.15; // far slower than commanded
            }
            sim.step(DT as f32);
            let acts = m.tick(
                t,
                &Sample {
                    position_mm: sim.pos,
                    held: true,
                    fault: None,
                },
            );
            if acts.first() == Some(&Action::Stop) {
                tripped = true;
                break;
            }
            sim.apply(t, &acts);
            if sim.vel > 0.0 {
                sim.vel = 0.15;
            }
            assert!(
                m.sub_block_s < cap,
                "submerged {} exceeded cap {}",
                m.sub_block_s,
                cap
            );
        }
        assert!(
            tripped,
            "watchdog or move timeout must retract a crawling rod"
        );
    }

    #[test]
    fn full_session_respects_cap_rest_and_ceiling_throughout() {
        for offset in [0.0_f32, 4.0] {
            let (mut m, cap) = machine(offset, 30.0);
            let mut sim = Sim::new(ORIGIN);
            let deepest_cmd = ORIGIN + 10.0_f32.max(5.0);
            let mut t = 0.0;
            let mut last_phase = DepthPhase::Arming;
            let mut rest_started: Option<f64> = None;
            let mut rests: Vec<(f64, f64)> = vec![];
            let mut max_block_sub = 0.0_f32;
            for _ in 0..200_000 {
                t += DT;
                sim.step(DT as f32);
                m.warmup_continue();
                let before = sim.moves.len();
                let acts = m.tick(
                    t,
                    &Sample {
                        position_mm: sim.pos,
                        held: true,
                        fault: None,
                    },
                );
                sim.apply(t, &acts);
                max_block_sub = max_block_sub.max(m.sub_block_s);
                for (_, p) in &sim.moves[before..] {
                    assert!(
                        *p <= deepest_cmd + 1e-3,
                        "commanded {p} beyond planned depth"
                    );
                }
                let ph = m.snapshot_phase();
                if ph == DepthPhase::Rest && last_phase != DepthPhase::Rest {
                    rest_started = Some(t);
                }
                if ph != DepthPhase::Rest && last_phase == DepthPhase::Rest {
                    if let Some(s) = rest_started.take() {
                        rests.push((s, t));
                    }
                }
                last_phase = ph;
                if m.is_done() {
                    break;
                }
            }
            assert!(m.is_done(), "session should finish (offset {offset})");
            assert!(m.outcome().completed);
            assert!(
                max_block_sub < cap,
                "submerged {max_block_sub} reached cap {cap}"
            );
            assert!(!rests.is_empty());
            for (s, e) in rests {
                assert!(e - s >= 20.0 - 2.0 * DT, "rest only {} s", e - s);
            }
        }
    }

    #[test]
    fn rest_after_a_block_is_at_least_three_times_the_time_submerged() {
        // a hold block submerges the user for several seconds: the rest must follow the rule
        let (mut m, _) = machine(0.0, 60.0); // cap 20 s → longer holds
        let mut sim = Sim::new(ORIGIN);
        let mut t = 0.0;
        let mut seen = false;
        let mut last_phase = DepthPhase::Arming;
        let mut sub_at_end = 0.0f32;
        for _ in 0..200_000 {
            t += DT;
            sim.step(DT as f32);
            let acts = m.tick(
                t,
                &Sample {
                    position_mm: sim.pos,
                    held: true,
                    fault: None,
                },
            );
            sim.apply(t, &acts);
            if m.snapshot_phase() == DepthPhase::Rest
                && last_phase != DepthPhase::Rest
                && m.rest_total > 20.0
            {
                sub_at_end = m.sub_block_s;
                assert!(m.rest_total >= 3.0 * sub_at_end - 1e-3);
                seen = true;
                break;
            }
            last_phase = m.snapshot_phase();
        }
        assert!(
            seen,
            "expected a rest longer than the 20 s floor (sub {sub_at_end})"
        );
    }

    #[test]
    fn hold_dwell_never_pushes_total_submerged_time_past_the_cap() {
        let l = lim();
        for breath in [12.0_f32, 18.0, 30.0, 60.0] {
            let cap = l.cap_s(breath);
            let t = Targets {
                osc_depth_mm: 6.0,
                hold_depth_mm: 8.0,
                hold_s: 99.0,
            };
            let Ok((blocks, _)) = plan_session(&l, &t, cap, 0.0) else {
                continue;
            };
            let hold = blocks
                .iter()
                .find(|b| b.kind == BlockKind::Hold)
                .unwrap()
                .clone();
            let total = l.out_time_s(hold.depth_mm) + hold.dwell_s + l.back_time_s(hold.depth_mm);
            assert!(
                total + MARGIN_S <= cap + 1e-3,
                "breath {breath}: {total} + margin > cap {cap}"
            );
        }
    }

    // ── task wrapper ──

    use tokio::sync::mpsc;

    fn cfg() -> Config {
        toml::from_str(
            r#"
            [actuator]
            variant = "12inch"
            [actuator.limits]
            max_velocity_mm_s = 400.0
            default_accel_g = 0.3
            default_motion_profile = "s_curve"
        "#,
        )
        .unwrap()
    }

    fn task_with_state() -> (
        DepthTask,
        Arc<RwLock<AppState>>,
        mpsc::Receiver<ActuatorCommand>,
    ) {
        let mut c = cfg();
        c.actuator.depth_trainer.progress_file = std::env::temp_dir()
            .join(format!(
                "rod-depth-task-{}-{}.json",
                std::process::id(),
                now_ms()
            ))
            .to_string_lossy()
            .into_owned();
        let state = Arc::new(RwLock::new(AppState::new("u".into(), 1)));
        let (tx, rx) = mpsc::channel(64);
        (DepthTask::new(state.clone(), tx, &c), state, rx)
    }

    /// Advance the paused clock in small steps, letting the task run in between.
    async fn pump(ms: u64) {
        for _ in 0..(ms / 10).max(1) {
            tokio::time::advance(Duration::from_millis(10)).await;
            for _ in 0..4 {
                tokio::task::yield_now().await;
            }
        }
    }

    async fn ready_state(state: &Arc<RwLock<AppState>>) {
        let mut st = state.write().await;
        st.set_mode(AppMode::Idle);
        st.actuator_connected = true;
        st.work_origin_mm = Some(ORIGIN);
        st.position_mm = ORIGIN;
        st.depth_progress.set_settings(30.0, 0.0).unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn start_is_refused_without_calibration_or_breath_hold() {
        let (task, state, _rx) = task_with_state();
        {
            let mut st = state.write().await;
            st.set_mode(AppMode::Idle);
            st.actuator_connected = true;
        }
        let (tx, rx) = mpsc::channel(8);
        let h = tokio::spawn(task.run(rx));
        tx.send(DepthControl::Start).await.unwrap();
        pump(200).await;
        assert_eq!(
            state.read().await.mode,
            AppMode::Idle,
            "no calibration → no session"
        );
        drop(tx);
        let _ = h.await;
    }

    #[tokio::test(start_paused = true)]
    async fn task_retracts_to_origin_when_the_switch_is_released() {
        let (task, state, mut rx) = task_with_state();
        ready_state(&state).await;
        let (tx, ctrl_rx) = mpsc::channel(8);
        let h = tokio::spawn(task.run(ctrl_rx));
        tx.send(DepthControl::Start).await.unwrap();
        pump(100).await;
        assert_eq!(state.read().await.mode, AppMode::DepthTrainer);
        assert!(state.read().await.depth.driving);
        assert!(matches!(
            rx.recv().await.unwrap(),
            ActuatorCommand::ServoOn(true)
        ));

        // hold the switch; keep the status "fresh" and walk the rod out
        let mut pos = ORIGIN;
        let mut first_move = false;
        for i in 0..400 {
            {
                let mut st = state.write().await;
                st.hand_switch = true;
                st.status_seq += 1;
                st.position_mm = pos;
            }
            pump(50).await;
            while let Ok(cmd) = rx.try_recv() {
                if let ActuatorCommand::MoveTo { pos_mm, .. } = cmd {
                    first_move = true;
                    pos = pos_mm.min(ORIGIN + 4.0);
                }
            }
            if first_move && pos > ORIGIN + 3.0 {
                let _ = i;
                break;
            }
        }
        assert!(
            first_move,
            "should have started moving with the switch held"
        );

        // release → Stop then MoveTo(origin)
        {
            let mut st = state.write().await;
            st.hand_switch = false;
            st.status_seq += 1;
            st.position_mm = ORIGIN + 4.0;
        }
        pump(60).await;
        let mut cmds = vec![];
        while let Ok(c) = rx.try_recv() {
            cmds.push(c);
        }
        let stop_at = cmds
            .iter()
            .position(|c| matches!(c, ActuatorCommand::Stop))
            .expect("Stop on release");
        match &cmds[stop_at + 1] {
            ActuatorCommand::MoveTo { pos_mm, .. } => assert_eq!(*pos_mm, ORIGIN),
            c => panic!("expected MoveTo(origin) after Stop, got {c:?}"),
        }
        assert!(state.read().await.depth.early_release);
        drop(tx);
        let _ = h.await;
    }

    #[tokio::test(start_paused = true)]
    async fn another_program_taking_the_mode_makes_the_trainer_retract_and_exit() {
        let (task, state, mut rx) = task_with_state();
        ready_state(&state).await;
        let (tx, ctrl_rx) = mpsc::channel(8);
        let h = tokio::spawn(task.run(ctrl_rx));
        tx.send(DepthControl::Start).await.unwrap();
        pump(100).await;
        assert_eq!(state.read().await.mode, AppMode::DepthTrainer);
        // rod is out beyond the origin when something else grabs the mode
        {
            let mut st = state.write().await;
            st.position_mm = ORIGIN + 6.0;
            st.status_seq += 1;
            st.set_mode(AppMode::Plumb);
        }
        pump(100).await;
        let mut cmds = vec![];
        while let Ok(c) = rx.try_recv() {
            cmds.push(c);
        }
        let i = cmds
            .iter()
            .position(|c| matches!(c, ActuatorCommand::Stop))
            .expect("Stop");
        assert!(
            matches!(&cmds[i + 1], ActuatorCommand::MoveTo { pos_mm, .. } if *pos_mm == ORIGIN)
        );
        drop(tx);
        let _ = h.await;
    }

    #[tokio::test(start_paused = true)]
    async fn stale_status_counts_as_not_held() {
        let (task, state, mut rx) = task_with_state();
        ready_state(&state).await;
        let (tx, ctrl_rx) = mpsc::channel(8);
        let h = tokio::spawn(task.run(ctrl_rx));
        tx.send(DepthControl::Start).await.unwrap();
        pump(100).await;
        let _ = rx.recv().await;
        // switch reads "held" but the poll counter never advances → link is dead
        state.write().await.hand_switch = true;
        pump(1000).await;
        let mut moved_out = false;
        while let Ok(c) = rx.try_recv() {
            if matches!(c, ActuatorCommand::MoveTo { pos_mm, .. } if pos_mm > ORIGIN + 0.5) {
                moved_out = true;
            }
        }
        assert!(
            !moved_out,
            "a stale 'held' reading must never drive the rod outward"
        );
        assert_eq!(
            state.read().await.depth.emergency_reason.as_deref(),
            Some("status data stale")
        );
        drop(tx);
        let _ = h.await;
    }
}
