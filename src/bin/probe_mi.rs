#[cfg(windows)]
mod probe {
    //! Elevated diagnostic probe for the native MI transport (ticket 16): drives
    //! the smart-charge and Acer-gaming adapters over the seam, printing raw
    //! readback rows and exercising the smart-charge write tuples one at a time.
    //!
    //! Test-time only — run ON the target laptop from an elevated shell. This
    //! probe DOES change the smart-charge cap state; it ends with the cap
    //! re-enabled (mask-2 tuple).

    use std::thread;
    use std::time::Duration;

    use nitro_tray::adapter::AdapterError;
    use nitro_tray::charge::{CLASS_NAME, ChargeRow, SmartChargeAdapter};
    use nitro_tray::wmi::{CLASS_NAME as WMI_CLASS_NAME, WmiAdapter};

    fn print_row(label: &str, row: Result<Option<ChargeRow>, AdapterError>) {
        match row {
            Ok(Some(row)) => println!(
                "{label}: list={} status=[{}]",
                row.function_list,
                row.status
                    .iter()
                    .map(|b| b.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            Ok(None) => println!("{label}: no row"),
            Err(e) => println!("{label}: ERROR {e:?}"),
        }
    }

    pub fn run() {
        println!("probe_mi: native MI transport (ticket 16)");
        let adapter = match SmartChargeAdapter::connect() {
            Ok(a) => a,
            Err(e) => {
                eprintln!("connect failed: {e:?}");
                std::process::exit(1);
            }
        };
        println!("connected (MI client + local session)");
        println!("  instance class: {CLASS_NAME}");

        println!("== {CLASS_NAME} readback row (battery 1, query 1) ==");
        print_row("row", adapter.read_row(1, 1));

        println!("== SetBatteryHealthControl tuples (status=1 enable) ==");
        for (battery, mask) in [(1u8, 1u8), (1u8, 2u8), (1u8, 3u8)] {
            println!("  set ({battery},{mask},1):");
            match adapter.write_tuple(battery, mask, 1) {
                Ok(rv) => println!("    ReturnValue={rv:?}"),
                Err(e) => println!("    ReturnValue: ERROR {e:?}"),
            }
            thread::sleep(Duration::from_millis(400));
            print_row("  row after", adapter.read_row(1, 1));
        }

        println!("== SetBatteryHealthControl (1,2,0) disable, then restore (1,2,1) ==");
        println!("  set (1,2,0):");
        let _ = adapter.write_tuple(1, 2, 0);
        thread::sleep(Duration::from_millis(400));
        print_row("  row after", adapter.read_row(1, 1));
        println!("  set (1,2,1) restore:");
        let _ = adapter.write_tuple(1, 2, 1);
        thread::sleep(Duration::from_millis(400));
        print_row("  row after", adapter.read_row(1, 1));

        println!("== {WMI_CLASS_NAME}: profile readback + fan ==");
        let wmi = match WmiAdapter::connect() {
            Ok(w) => w,
            Err(e) => {
                println!("    {WMI_CLASS_NAME}: ERROR {e:?}");
                println!("probe_mi complete");
                return;
            }
        };
        println!("    instance class: {WMI_CLASS_NAME}");
        match wmi.get_platform_profile() {
            Ok(profile) => println!(
                "    GetGamingMiscSetting(0x0B): gmOutput=0x{profile:X} (profile {profile})"
            ),
            Err(e) => println!("    GetGamingMiscSetting: ERROR {e:?}"),
        }
        match wmi.get_fan_behavior() {
            Ok(value) => println!("    GetGamingFanBehavior: gmOutput={value} (0x{value:X})"),
            Err(e) => println!("    GetGamingFanBehavior: ERROR {e:?}"),
        }
        println!("probe_mi complete");
    }
}

#[cfg(windows)]
fn main() {
    probe::run();
}

#[cfg(target_os = "linux")]
fn main() {}
