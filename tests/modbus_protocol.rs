//! Integration coverage for the Modbus packing/parsing over the public
//! `modbus::protocol` surface (the bytes that hit the wire).

use rod::config::MotionProfile;
use rod::modbus::protocol::{
    parse_status_block, Dss1, MoveCommand, Stat, MOVE_BLOCK_LEN, STATUS_BLOCK_LEN,
};

#[test]
fn move_command_is_nine_registers_high_word_first() {
    // 250.00 mm = 25000 (0x61A8), 400 mm/s = 40000 (0x9C40), 0.3 G = 30.
    let cmd = MoveCommand::new(25000, 40000, 30, MotionProfile::SCurve);
    let regs = cmd.to_registers();
    assert_eq!(regs.len(), MOVE_BLOCK_LEN as usize);
    assert_eq!([regs[0], regs[1]], [0x0000, 0x61A8]); // PCMD i32, hi word first
    assert_eq!([regs[2], regs[3]], [0x0000, 0x000A]); // position band = 10
    assert_eq!([regs[4], regs[5]], [0x0000, 0x9C40]); // VCMD u32
    assert_eq!(regs[6], 30); // ACMD
    assert_eq!(regs[7], 0); // push current
                            // CTLF MOD bits are 0: this controller faults (0xA3) on bits 4–7, so every
                            // profile emits trapezoid. See `ctlf_for_profile`.
    assert_eq!(regs[8], 0x00);
}

#[test]
fn status_block_parses_homed_servo_on_position() {
    let dss1 = Dss1::HEND.bits() | Dss1::SV.bits() | Dss1::PWR.bits();
    let stat = Stat::SON.bits() | Stat::SV.bits() | Stat::HEND.bits();
    let regs = [
        0x0001,
        0x86A0, // PNOW = 100000 (=1000.00 mm... here just an i32)
        0,      // ALMC
        0,      // DIPM
        0,      // DIPO
        dss1,   // DSS1
        0,      // DSS2
        0,      // DSSE
        (stat >> 16) as u16,
        stat as u16, // STAT
    ];
    assert_eq!(regs.len(), STATUS_BLOCK_LEN as usize);
    let s = parse_status_block(&regs).unwrap();
    assert_eq!(s.pnow, 100_000);
    assert_eq!(s.position_mm(), 1000.0);
    assert!(s.home_complete());
    assert!(s.servo_on());
    assert!(!s.major_alarm());
}

#[test]
fn alarm_code_flags_major_alarm() {
    let mut regs = [0u16; 10];
    regs[2] = 0x00B0; // ALMC nonzero
    let s = parse_status_block(&regs).unwrap();
    assert_eq!(s.almc, 0x00B0);
    assert!(s.major_alarm());
}
