//! Elevated diagnostic probe for the Acer WMI gaming interface (ticket 05).
//!
//! Test-time only — run ON the target laptop from an elevated shell (the
//! manifest requires administrator). Never shipped or run by the app itself.
//!
//! Flow: connect to `ROOT\WMI`/`AcerGamingFunction`, read the current
//! platform profile, set each of Quiet/Balanced/Performance/Eco with a
//! 300 ms pause and read back after each, restore the original profile, then
//! set fan behavior to auto and read it back raw.

use std::thread;
use std::time::Duration;

use nitro_tray::wmi::{
    WmiAdapter, PROFILE_BALANCED, PROFILE_ECO, PROFILE_PERFORMANCE, PROFILE_QUIET,
};

fn main() {
    let adapter = match WmiAdapter::connect() {
        Ok(adapter) => adapter,
        Err(e) => {
            eprintln!("connect failed: {e:?}");
            std::process::exit(1);
        }
    };
    println!("connected to ROOT\\WMI / AcerGamingFunction");

    let original = match adapter.get_platform_profile() {
        Ok(profile) => profile,
        Err(e) => {
            eprintln!("initial profile read failed: {e:?}");
            std::process::exit(1);
        }
    };
    println!("current platform profile: {original} (0x{original:X})");

    let steps = [
        (PROFILE_QUIET, "quiet"),
        (PROFILE_BALANCED, "balanced"),
        (PROFILE_PERFORMANCE, "performance"),
        (PROFILE_ECO, "eco"),
    ];
    for (value, name) in steps {
        match adapter.set_platform_profile(value) {
            Ok(()) => println!("set profile {name} ({value}): ok"),
            Err(e) => {
                println!("set profile {name} ({value}): FAILED {e:?}");
                continue;
            }
        }
        thread::sleep(Duration::from_millis(300));
        match adapter.get_platform_profile() {
            Ok(readback) => println!("  readback {name}: {readback} (0x{readback:X})"),
            Err(e) => println!("  readback {name}: FAILED {e:?}"),
        }
    }

    println!("restoring original profile {original} (0x{original:X})");
    match adapter.set_platform_profile(original) {
        Ok(()) => println!("restore ok"),
        Err(e) => eprintln!("restore FAILED {e:?}"),
    }

    match adapter.set_fan_auto() {
        Ok(()) => println!("set fan auto (0x00410009): ok"),
        Err(e) => println!("set fan auto: FAILED {e:?}"),
    }
    thread::sleep(Duration::from_millis(300));
    match adapter.get_fan_behavior() {
        Ok(value) => println!("fan behavior readback: {value} (0x{value:X})"),
        Err(e) => println!("fan behavior readback: FAILED {e:?}"),
    }
}
