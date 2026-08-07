//! Elevated diagnostic for the in-process power wrapper (ticket 04).
//!
//! Prints the active plan, ensures the four Nitro plans exist (idempotent —
//! safe to run twice), and lists their names plus the tuned CPU min/max/boost
//! read back via `PowerReadACValueIndex`. For on-device verification only;
//! not run in this environment.

use nitro_tray::power::{
    boost_mode_index, cpu_tuning, read_ac_index, PowerApi, NITRO_PLANS,
};
use windows_sys::Win32::System::SystemServices::{
    GUID_PROCESSOR_PERF_BOOST_MODE, GUID_PROCESSOR_THROTTLE_MAXIMUM,
    GUID_PROCESSOR_THROTTLE_MINIMUM,
};

fn guid_str(guid: &windows_sys::core::GUID) -> String {
    let b = guid.data4;
    format!(
        "{:08x}-{:04x}-{:04x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        guid.data1, guid.data2, guid.data3, b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]
    )
}

fn main() {
    match PowerApi::active_plan_name() {
        Ok(name) => println!("active plan: {name}"),
        Err(e) => println!("active plan: ERROR {e:?}"),
    }

    match PowerApi::ensure_nitro_plans() {
        Ok(()) => println!("ensure_nitro_plans: OK (idempotent, safe to re-run)"),
        Err(e) => println!("ensure_nitro_plans: ERROR {e:?}"),
    }

    println!("boost mode registry GUIDs:");
    println!("  disabled:  {}", guid_str(&nitro_tray::power::BOOST_MODE_DISABLED_VALUE_GUID));
    println!("  enabled:   {}", guid_str(&nitro_tray::power::BOOST_MODE_ENABLED_VALUE_GUID));
    println!("  aggressive:{}", guid_str(&nitro_tray::power::BOOST_MODE_AGGRESSIVE_VALUE_GUID));

    let profiles = [
        nitro_tray::policy::Profile::Quiet,
        nitro_tray::policy::Profile::Balanced,
        nitro_tray::policy::Profile::Performance,
        nitro_tray::policy::Profile::Eco,
    ];
    for (profile, name) in profiles.iter().zip(NITRO_PLANS) {
        let expected = cpu_tuning(*profile);
        println!("plan {name}: expected min={} max={} boost={}", expected.min_percent, expected.max_percent, boost_mode_index(expected.boost));
        match PowerApi::find_plan(name) {
            Ok(Some(guid)) => {
                println!("  found: {}", guid_str(&guid));
                for (label, setting) in [
                    ("min", GUID_PROCESSOR_THROTTLE_MINIMUM),
                    ("max", GUID_PROCESSOR_THROTTLE_MAXIMUM),
                    ("boost", GUID_PROCESSOR_PERF_BOOST_MODE),
                ] {
                    match read_ac_index(&guid, &setting) {
                        Ok(v) => println!("  read ac {label}: {v}"),
                        Err(e) => println!("  read ac {label}: ERROR {e:?}"),
                    }
                }
            }
            Ok(None) => println!("  MISSING"),
            Err(e) => println!("  ERROR {e:?}"),
        }
    }
}
