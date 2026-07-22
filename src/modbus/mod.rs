//! Modbus RTU layer: protocol packing/parsing (`protocol`) and the serial
//! driver that is the sole owner of the port (`driver`).

pub mod driver;
pub mod protocol;
