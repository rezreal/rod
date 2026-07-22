//! IAI controller Modbus protocol: register/coil addresses, status bitfields,
//! and PDU-level packing/parsing. Ported from `rezreal/knock-rod`
//! (`knockRodProtocol.ts`) — see SPEC §9 / §9.1.
//!
//! Unlike knock-rod (which hand-builds raw frames over WebSerial and computes
//! its own CRC), we work at the **PDU level**: `tokio-modbus`'s RTU codec owns
//! the slave address and the CRC16. So everything here is just the register
//! values — addresses, the `[u16; 9]` move payload, and decoding a status block
//! returned as `Vec<u16>`. No CRC, no byte slicing.

use crate::config::MotionProfile;

// A tiny bitflags substitute so we don't pull in the `bitflags` crate for four
// small flag sets. Generates a newtype over an integer with `contains`,
// `from_bits_truncate`, and bitwise helpers. Defined before first use.
macro_rules! bitflags_like {
    (
        $(#[$meta:meta])*
        pub struct $name:ident : $ty:ty { $(const $flag:ident = $val:expr;)* }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub struct $name(pub $ty);
        impl $name {
            $(pub const $flag: $name = $name($val);)*
            pub const fn from_bits_truncate(bits: $ty) -> Self { $name(bits) }
            pub const fn bits(self) -> $ty { self.0 }
            pub const fn contains(self, other: Self) -> bool { (self.0 & other.0) == other.0 }
            pub const fn is_empty(self) -> bool { self.0 == 0 }
        }
    };
}

// ───────────────────────────── Register addresses ─────────────────────────────

/// Status block base: PNOW, ALMC, DIPM, DIPO, DSS1, DSS2, DSSE, STAT (10 regs).
pub const REG_STATUS_BLOCK: u16 = 0x9000;
pub const STATUS_BLOCK_LEN: u16 = 10;

/// Present alarm code (1 register).
pub const REG_ALARM_CODE: u16 = 0x9002;

/// Movement command block base (9 registers via FC 0x10).
pub const REG_MOVE_BLOCK: u16 = 0x9900;
pub const MOVE_BLOCK_LEN: u16 = 9;

// ───────────────────────────────── Coils ──────────────────────────────────────

/// Safety-speed enable.
pub const COIL_SAFETY_SPEED: u16 = 0x0401;
/// SON — servo ON/OFF.
pub const COIL_SERVO: u16 = 0x0403;
/// ALRS — alarm reset (edge: FF00 then 0000).
pub const COIL_ALARM_RESET: u16 = 0x0407;
/// BKRL — brake forced release.
pub const COIL_BRAKE_RELEASE: u16 = 0x0408;
/// HOME — home-return (edge).
pub const COIL_HOME: u16 = 0x040B;
/// PMSS — PIO/Modbus switch (FF00 = Modbus commands enabled).
pub const COIL_PIO_MODBUS: u16 = 0x0427;
/// STOP — deceleration stop (edge: FF00).
pub const COIL_DECEL_STOP: u16 = 0x042C;

// ─────────────────────────────── Status bitfields ─────────────────────────────

bitflags_like! {
    /// System status register (STAT, part of the 0x9000 block).
    pub struct Stat: u32 {
        const MPOW = 1 << 0; // drive source on
        const SON  = 1 << 1; // servo command status
        const SV   = 1 << 2; // servo status
        const HEND = 1 << 3; // home-return complete
        const RMDS = 1 << 4; // MANU mode
    }
}

bitflags_like! {
    /// Device status register 1 (DSS1, 0x9005).
    pub struct Dss1: u16 {
        const PEND = 1 << 3;  // positioning complete
        const HEND = 1 << 4;  // home-return complete
        const STP  = 1 << 5;  // pause command active
        const BKRL = 1 << 7;  // brake released
        const ABER = 1 << 8;  // absolute error
        const ALML = 1 << 9;  // minor failure
        const ALMH = 1 << 10; // major failure
        const SV   = 1 << 12; // servo on
        const PWR  = 1 << 13; // controller ready
        const SFTY = 1 << 14; // safety speed enabled
        const EMGS = 1 << 15; // emergency stop actuated
    }
}

bitflags_like! {
    /// Expansion device status register (DSSE, 0x9007).
    pub struct Dsse: u16 {
        const MOVE = 1 << 5;  // moving
        const PMSS = 1 << 8;  // PIO/Modbus switch status
        const PSNS = 1 << 9;  // excitation detection complete
        const PUSH = 1 << 10; // push-motion in progress
        const GHMS = 1 << 11; // home-return in progress
        const RMDS = 1 << 13; // MANU mode
        const MPUV = 1 << 14; // motor voltage low
        const EMGP = 1 << 15; // emergency stop input on
    }
}

/// Parsed contents of the 0x9000 status block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatusBlock {
    /// Present position PNOW in 0.01 mm units (i32).
    pub pnow: i32,
    /// Present alarm code (0 == normal).
    pub almc: u16,
    /// Input port monitor (DIPM) — bit 0 is the palm/hand switch.
    pub dipm: u16,
    /// Output port monitor (DIPO).
    pub dipo: u16,
    pub dss1: Dss1,
    pub dss2: u16,
    pub dsse: Dsse,
    pub stat: Stat,
}

impl StatusBlock {
    /// PNOW converted to mm.
    pub fn position_mm(&self) -> f32 {
        self.pnow as f32 / 100.0
    }
    /// Home-return complete (DSS1.HEND, the authoritative bit knock-rod polls).
    pub fn home_complete(&self) -> bool {
        self.dss1.contains(Dss1::HEND)
    }
    pub fn positioning_complete(&self) -> bool {
        self.dss1.contains(Dss1::PEND)
    }
    pub fn moving(&self) -> bool {
        self.dsse.contains(Dsse::MOVE)
    }
    /// Push-motion in progress (DSSE.PUSH): the rod is pressing against a stop.
    /// This is the contact signal a push-to-contact calibration watches for.
    pub fn pushing(&self) -> bool {
        self.dsse.contains(Dsse::PUSH)
    }
    pub fn servo_on(&self) -> bool {
        self.dss1.contains(Dss1::SV)
    }
    /// Hand/palm switch wired to the controller's PIO input (DIPM bit 0).
    /// Readable even in Modbus mode; the bridge only monitors it.
    pub fn hand_switch(&self) -> bool {
        self.dipm & 0x0001 != 0
    }
    pub fn major_alarm(&self) -> bool {
        self.dss1.contains(Dss1::ALMH) || self.almc != 0
    }
}

/// Decode the 10-register status block returned by
/// `read_holding_registers(0x9000, 10)`.
///
/// Word layout (high word first, matching knock-rod's big-endian `DataView`):
/// `[PNOW_hi, PNOW_lo, ALMC, DIPM, DIPO, DSS1, DSS2, DSSE, STAT_hi, STAT_lo]`.
pub fn parse_status_block(regs: &[u16]) -> Result<StatusBlock, ProtocolError> {
    if regs.len() < STATUS_BLOCK_LEN as usize {
        return Err(ProtocolError::ShortStatusBlock(regs.len()));
    }
    let pnow = ((regs[0] as i32) << 16) | regs[1] as i32;
    let stat = ((regs[8] as u32) << 16) | regs[9] as u32;
    Ok(StatusBlock {
        pnow,
        almc: regs[2],
        dipm: regs[3],
        dipo: regs[4],
        dss1: Dss1::from_bits_truncate(regs[5]),
        dss2: regs[6],
        dsse: Dsse::from_bits_truncate(regs[7]),
        stat: Stat::from_bits_truncate(stat),
    })
}

// ───────────────────────────── Movement command ──────────────────────────────

/// CTLF "MOD" profile bits for the numerical-move command.
///
/// **Hardware finding (this controller):** the MOD bits knock-rod uses —
/// `MOD0 = 1<<6` (S-motion) and `MOD1 = 1<<7` (primary-delay filter) — are
/// *rejected* here. Setting **any** of CTLF bits 4–7 makes the controller
/// abort the move with alarm `0x00A3` (command-data error) and drop the servo,
/// even on a zero-distance move. Verified empirically by sweeping every CTLF
/// bit over the debug console: only bits 0–3 / 8+ are accepted, and none of the
/// accepted values produce clean S-curve motion (they no-op or stall the move).
///
/// So on this actuator only **trapezoid** (CTLF MOD = 0) is usable. We therefore
/// emit 0 for every profile; the [`MotionProfile`] selection is preserved in the
/// config/API surface but has no MOD effect here. Re-introduce the MOD bits only
/// for a controller model documented to accept them.
fn ctlf_for_profile(p: MotionProfile) -> u16 {
    match p {
        MotionProfile::Trapezoid | MotionProfile::SCurve | MotionProfile::Filter => 0,
    }
}

/// CTLF PUSH bit: switches the move from positioning to push-motion. With it
/// set, the controller advances toward `target_pos` but caps thrust at
/// `push_current`; on contact it stops, holds, and asserts DSSE.PUSH. This is
/// the low bit of CTLF in the IAI numerical-movement command — verify against
/// your controller's instruction manual if behaviour looks off.
const CTLF_PUSH: u16 = 1 << 0;

/// Parameters for a numerical-value movement command (FC 0x10 @ 0x9900).
///
/// All integer units are the controller's native scaling:
/// position in 0.01 mm, velocity in 0.01 mm/s, accel in 0.01 G.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MoveCommand {
    /// PCMD — target position (0.01 mm).
    pub target_pos: i32,
    /// Position band (0.01 mm). knock-rod default is 10 (= 0.1 mm).
    pub position_band: i32,
    /// VCMD — speed (0.01 mm/s).
    pub velocity: u32,
    /// ACMD — accel/decel (0.01 G).
    pub accel: u16,
    /// Push-current limit (0 = no limit).
    pub push_current: u16,
    /// CTLF control flags (profile MOD bits, etc.).
    pub ctlf: u16,
}

impl MoveCommand {
    /// Build a move command from engineering units. Mirrors knock-rod's
    /// `numericalValueMovementCommand` defaults (band=10, push=0).
    pub fn new(
        target_pos_001mm: i32,
        velocity_001mm_s: u32,
        accel_001g: u16,
        profile: MotionProfile,
    ) -> Self {
        MoveCommand {
            target_pos: target_pos_001mm,
            position_band: 10,
            velocity: velocity_001mm_s,
            accel: accel_001g,
            push_current: 0,
            ctlf: ctlf_for_profile(profile),
        }
    }

    /// Build a **push-motion** command (CTLF.PUSH set). Identical to [`new`]
    /// except thrust is capped at `push_current_pct` (percent of rated thrust)
    /// so the rod presses into a stop instead of faulting on a hard target.
    /// Used by the contact-calibration routine.
    pub fn new_push(
        target_pos_001mm: i32,
        velocity_001mm_s: u32,
        accel_001g: u16,
        push_current_pct: u16,
        profile: MotionProfile,
    ) -> Self {
        MoveCommand {
            target_pos: target_pos_001mm,
            position_band: 10,
            velocity: velocity_001mm_s,
            accel: accel_001g,
            push_current: push_current_pct,
            ctlf: ctlf_for_profile(profile) | CTLF_PUSH,
        }
    }

    /// Pack into the 9 holding registers written at 0x9900, high word first.
    pub fn to_registers(&self) -> [u16; 9] {
        let pos = self.target_pos as u32;
        let band = self.position_band as u32;
        let vel = self.velocity;
        [
            (pos >> 16) as u16,
            pos as u16,
            (band >> 16) as u16,
            band as u16,
            (vel >> 16) as u16,
            vel as u16,
            self.accel,
            self.push_current,
            self.ctlf,
        ]
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    #[error("status block too short: got {0} registers, need {STATUS_BLOCK_LEN}")]
    ShortStatusBlock(usize),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn move_command_packs_high_word_first() {
        // 200.00 mm -> 20000 (0x4E20); velocity 30000 (0x7530); accel 30; trapezoid.
        let cmd = MoveCommand::new(20000, 30000, 30, MotionProfile::Trapezoid);
        let regs = cmd.to_registers();
        assert_eq!(regs[0], 0x0000); // PCMD hi
        assert_eq!(regs[1], 0x4E20); // PCMD lo
        assert_eq!(regs[2], 0x0000); // band hi
        assert_eq!(regs[3], 10); // band lo (default 0.1mm)
        assert_eq!(regs[4], 0x0000); // VCMD hi
        assert_eq!(regs[5], 0x7530); // VCMD lo
        assert_eq!(regs[6], 30); // ACMD
        assert_eq!(regs[7], 0); // push current
        assert_eq!(regs[8], 0); // CTLF (trapezoid)
    }

    #[test]
    fn ctlf_profile_bits() {
        // This controller rejects the MOD bits (alarm 0xA3 on bits 4–7), so all
        // profiles emit trapezoid CTLF = 0. See `ctlf_for_profile`.
        assert_eq!(MoveCommand::new(0, 1, 1, MotionProfile::Trapezoid).ctlf, 0);
        assert_eq!(MoveCommand::new(0, 1, 1, MotionProfile::SCurve).ctlf, 0);
        assert_eq!(MoveCommand::new(0, 1, 1, MotionProfile::Filter).ctlf, 0);
    }

    #[test]
    fn push_command_sets_push_current_and_ctlf_push_bit() {
        // Push-motion at 5% thrust, trapezoid profile.
        let cmd = MoveCommand::new_push(30000, 200, 10, 5, MotionProfile::Trapezoid);
        let regs = cmd.to_registers();
        assert_eq!(regs[7], 5); // push-current limit
        assert_eq!(regs[8] & 1, 1); // CTLF.PUSH set

        // Profile MOD bits are 0 here, so push CTLF is just the PUSH bit.
        let scurve = MoveCommand::new_push(0, 1, 1, 5, MotionProfile::SCurve);
        assert_eq!(scurve.ctlf, 0x01);
    }

    #[test]
    fn negative_position_roundtrips_through_registers() {
        let cmd = MoveCommand::new(-12345, 100, 5, MotionProfile::SCurve);
        let regs = cmd.to_registers();
        let packed = ((regs[0] as u32) << 16 | regs[1] as u32) as i32;
        assert_eq!(packed, -12345);
    }

    #[test]
    fn parse_status_block_decodes_fields() {
        // PNOW = 15000 (0x00003A98), ALMC=0, DIPM=1 (palm switch), DIPO=0,
        // DSS1 = HEND|SV|PWR, DSS2=0, DSSE=0, STAT = SON|SV|HEND.
        let dss1 = Dss1::HEND.bits() | Dss1::SV.bits() | Dss1::PWR.bits();
        let stat = Stat::SON.bits() | Stat::SV.bits() | Stat::HEND.bits();
        let regs = [
            0x0000,
            0x3A98, // PNOW
            0,      // ALMC
            1,      // DIPM
            0,      // DIPO
            dss1,   // DSS1
            0,      // DSS2
            0,      // DSSE
            (stat >> 16) as u16,
            stat as u16, // STAT
        ];
        let s = parse_status_block(&regs).unwrap();
        assert_eq!(s.pnow, 15000);
        assert_eq!(s.position_mm(), 150.0);
        assert!(s.home_complete());
        assert!(s.servo_on());
        assert!(!s.major_alarm());
        assert_eq!(s.dipm & 1, 1);
        assert!(s.stat.contains(Stat::SON));
    }

    #[test]
    fn short_status_block_errors() {
        assert!(parse_status_block(&[0; 4]).is_err());
    }
}
