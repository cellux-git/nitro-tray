//! Elevated diagnostic for the smart-charge adapter: connect to
//! `BatteryControl`, read the current state, toggle the 80% cap ON, read
//! back, toggle OFF, read back, then restore the original state.
//!
//! On-device only: must run elevated (the manifest requires administrator).
//! `cargo run --bin probe_charge` and observe the printed state transitions.

use nitro_tray::adapter::AdapterError;
use nitro_tray::charge::SmartChargeAdapter;

fn report(label: &str, result: Result<bool, AdapterError>) {
    match result {
        Ok(state) => println!("{label}: enabled={state}"),
        Err(e) => println!("{label}: ERROR {e:?}"),
    }
}

fn main() {
    println!("nitro-tray probe: smart charge (BatteryControl, ROOT\\WMI)");
    println!("this probe toggles the 80% charge cap; run elevated");
    let adapter = match SmartChargeAdapter::connect() {
        Ok(a) => a,
        Err(e) => {
            println!("connect: ERROR {e:?}");
            std::process::exit(1);
        }
    };
    println!("connect: ok");
    let original = match adapter.is_enabled() {
        Ok(state) => {
            println!("original state: enabled={state}");
            state
        }
        Err(e) => {
            println!("is_enabled (original): ERROR {e:?}");
            false
        }
    };
    match adapter.set_enabled(true) {
        Ok(()) => println!("set_enabled(true): ok"),
        Err(e) => println!("set_enabled(true): ERROR {e:?}"),
    }
    report("readback after ON", adapter.is_enabled());
    match adapter.set_enabled(false) {
        Ok(()) => println!("set_enabled(false): ok"),
        Err(e) => println!("set_enabled(false): ERROR {e:?}"),
    }
    report("readback after OFF", adapter.is_enabled());
    match adapter.set_enabled(original) {
        Ok(()) => println!("set_enabled({original}) restore: ok"),
        Err(e) => println!("set_enabled({original}) restore: ERROR {e:?}"),
    }
    report("readback after restore", adapter.is_enabled());
    println!("probe complete");
}
