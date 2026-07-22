//! External actuator devices reached over BLE-central (in addition to the
//! Modbus rod). Currently the DG-LAB Coyote e-stim box; this is where a future
//! buttplug bridge or other native device drivers would live too (see
//! docs/buttplug-integration.md).

pub mod coyote;

pub use coyote::CoyoteControl;
