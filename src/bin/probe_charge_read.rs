#[cfg(windows)]
mod probe {
    //! Elevated diagnostic for the smart-charge adapter — READ ONLY. Connects to
    //! `BatteryControl` and prints the current 80% cap state plus the full
    //! read-only health-status snapshot (battery × query sweep). Never writes.
    //!
    //! On-device only: must run elevated (the manifest requires administrator).
    //! `cargo run --bin probe_charge_read` and read the printed state.

    use nitro_tray::adapter::{AdapterError, WMI_NAMESPACE};
    use nitro_tray::charge::{CLASS_NAME, ChargeRow, SmartChargeAdapter};

    fn print_row(label: &str, row: Result<Option<ChargeRow>, AdapterError>) {
        match row {
            Ok(Some(row)) => println!(
                "{label}: list={} status=[{}] return=[{}]",
                row.function_list,
                row.status
                    .iter()
                    .map(u8::to_string)
                    .collect::<Vec<_>>()
                    .join(","),
                row.return_value
                    .map(|rv| rv.to_string())
                    .unwrap_or_default()
            ),
            Ok(None) => println!("{label}: no row"),
            Err(e) => println!("{label}: ERROR {e:?}"),
        }
    }

    pub fn run() {
        println!("nitro-tray probe: smart charge ({CLASS_NAME}, {WMI_NAMESPACE}) — read only");
        let adapter = match SmartChargeAdapter::connect() {
            Ok(a) => a,
            Err(e) => {
                println!("connect: ERROR {e:?}");
                std::process::exit(1);
            }
        };
        println!("connect: ok");
        match adapter.is_enabled() {
            Ok(state) => println!("smart charge (80% cap): enabled={state}"),
            Err(e) => println!("is_enabled: ERROR {e:?}"),
        }

        println!("== GetBatteryHealthControlStatus sweep (battery 0..3, query 0..3) ==");
        for battery in 0..4u8 {
            for query in 0..4u8 {
                print_row(
                    &format!("  b{battery} q{query}"),
                    adapter.read_row(battery, query),
                );
            }
        }
        println!("probe complete");
    }
}

#[cfg(windows)]
fn main() {
    probe::run();
}

#[cfg(target_os = "linux")]
fn main() {}
