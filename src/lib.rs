//! Nitro Tray — a single portable exe that keeps an Acer Nitro laptop in a
//! coherent power state from the system tray.
//!
//! All hardware/OS control is in-process (COM/WMI, power APIs, HID feature
//! reports). No external process is ever spawned.
//!
//! Platform split (linux-port ticket 02): the policy engine, config/state,
//! opcode tables, adapter seams (`MiTransport`/`HidTransport`/`PlanApi`) and
//! the circuit-breaker + recovery machinery are OS-independent and build on
//! both platforms. The Windows transports (mi.dll WMI, HID SetupDi,
//! power APIs) and the tray/hotkey/task modules are `#[cfg(windows)]`; on
//! Linux the seams report "unavailable" and the power-state reader runs on
//! `/sys/class/power_supply`, so `cargo check --target x86_64-unknown-linux-gnu`
//! succeeds with no Windows imports.

pub mod adapter;
pub mod app;
pub mod charge;
pub mod config;
pub mod hid;
#[cfg(windows)]
pub mod hotkey;
pub mod log;
pub mod mi;
pub mod policy;
pub mod power;
pub mod power_state;
#[cfg(windows)]
pub mod task;
pub mod timers;
pub mod transport;
pub mod tray_model;
#[cfg(windows)]
pub mod tray;
pub mod wiring;
pub mod wmi;

#[cfg(test)]
pub mod testing;
