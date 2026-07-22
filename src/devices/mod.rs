//! External actuator devices reached over BLE-central (in addition to the
//! Modbus rod). Currently the DG-LAB Coyote e-stim box and the Hismith PiuPiu
//! lube launcher; this is where a future buttplug bridge or other native
//! device drivers would live too (see docs/buttplug-integration.md).

pub mod coyote;
pub mod piupiu;

pub use coyote::CoyoteControl;
pub use piupiu::PiuPiuControl;

/// Sidecar file holding the persisted autoconnect flag for a device (e.g.
/// `autoconnect-coyote`), next to the binary — same convention as
/// `max-depth-mm` (see `modbus::driver::load_max_depth`).
fn autoconnect_file(name: &str) -> String {
    format!("autoconnect-{name}")
}

/// Read the persisted autoconnect flag for `name`. Falls back to `default`
/// (the config.toml `enable` value) when the sidecar file is absent or
/// unparseable — i.e. `default` is only ever the *first-boot* behavior.
pub fn load_autoconnect(name: &str, default: bool) -> bool {
    std::fs::read_to_string(autoconnect_file(name))
        .ok()
        .and_then(|s| s.trim().parse::<bool>().ok())
        .unwrap_or(default)
}

/// Persist the autoconnect flag for `name` so it survives a reboot.
/// Best-effort: a failed write only means the setting won't survive a reboot,
/// not that the live toggle failed.
pub fn persist_autoconnect(name: &str, enabled: bool) {
    if let Err(e) = std::fs::write(autoconnect_file(name), enabled.to_string()) {
        tracing::warn!(error = %e, device = name, "failed to persist autoconnect; value is in-memory only");
    }
}
