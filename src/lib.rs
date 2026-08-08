//! Nitro Tray — a single portable exe that keeps an Acer Nitro laptop in a
//! coherent power state from the system tray.
//!
//! All hardware/OS control is in-process (COM/WMI, power APIs, HID feature
//! reports). No external process is ever spawned.

pub mod adapter;
pub mod app;
pub mod charge;
pub mod config;
pub mod hid;
pub mod hotkey;
pub mod log;
pub mod mi;
pub mod policy;
pub mod power;
pub mod power_state;
pub mod task;
pub mod timers;
pub mod transport;
pub mod tray;
pub mod wmi;

#[cfg(test)]
pub mod testing;
