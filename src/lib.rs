//! `rod` library crate — makes an IAI linear actuator
//! (Modbus RTU) appear as an Ohdoki "Handy" FW4 device over the shared
//! `RpcMessage` protocol. See `SPEC.md`.
//!
//! The binary (`main.rs`) wires these modules into the runtime task graph; the
//! integration tests in `tests/` exercise the public surface.

pub mod config;
pub mod debug;
pub mod devices;
pub mod modbus;
pub mod modes;
pub mod rpc;
pub mod sensors;
pub mod shaper;
pub mod sscp;
pub mod state;
pub mod telemetry;
pub mod translator;
pub mod transport;
