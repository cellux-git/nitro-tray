//! Elevated diagnostic probe for the native MI transport (ticket 16): drives
//! `mi::MiConnection` directly against `BatteryControl` and
//! `AcerGamingFunction`, printing raw readback rows and exercising the
//! smart-charge write tuples one at a time.
//!
//! Test-time only — run ON the target laptop from an elevated shell. This
//! probe DOES change the smart-charge cap state; it ends with the cap
//! re-enabled (mask-2 tuple).

use std::thread;
use std::time::Duration;

use nitro_tray::mi::MiConnection;

const NAMESPACE: &str = "ROOT\\WMI";

fn read_row(
    connection: &MiConnection,
    battery: u8,
    query: u8,
) -> Result<(u32, Vec<u8>), String> {
    let instance = connection.enumerate_first_instance(NAMESPACE, "BatteryControl").map_err(|e| e.to_string())?;
    println!("    instance class: {}", instance.class_name());
    let mut input = connection.new_instance("BatteryControl").map_err(|e| e.to_string())?;
    input.add_u8("uBatteryNo", battery).map_err(|e| e.to_string())?;
    input.add_u8("uFunctionQuery", query).map_err(|e| e.to_string())?;
    input.add_u8_array("uReserved", &[0, 0]).map_err(|e| e.to_string())?;
    let out = connection
        .invoke(NAMESPACE, &instance, "GetBatteryHealthControlStatus", &input)
        .map_err(|e| e.to_string())?;
    let Some(result) = out else { return Err("no out-params".into()) };
    let list = result.get_u32("uFunctionList").map_err(|e| e.to_string())?;
    let statuses = result.get_u8_array("uFunctionStatus").map_err(|e| e.to_string())?;
    Ok((list.unwrap_or(0), statuses.unwrap_or_default()))
}

fn set_tuple(connection: &MiConnection, battery: u8, mask: u8, status: u8) -> Result<Option<u32>, String> {
    let instance = connection.enumerate_first_instance(NAMESPACE, "BatteryControl").map_err(|e| e.to_string())?;
    let mut input = connection.new_instance("BatteryControl").map_err(|e| e.to_string())?;
    input.add_u8("uBatteryNo", battery).map_err(|e| e.to_string())?;
    input.add_u8("uFunctionMask", mask).map_err(|e| e.to_string())?;
    input.add_u8("uFunctionStatus", status).map_err(|e| e.to_string())?;
    input.add_u8_array("uReservedIn", &[0, 0, 0, 0, 0]).map_err(|e| e.to_string())?;
    let out = connection
        .invoke(NAMESPACE, &instance, "SetBatteryHealthControl", &input)
        .map_err(|e| e.to_string())?;
    let Some(result) = out else { return Ok(None) };
    let rv = result.get_u32("ReturnValue").map_err(|e| e.to_string())?;
    let u_return = result.get_u32("uReturn").map_err(|e| e.to_string())?;
    println!("    ReturnValue={rv:?} uReturn={u_return:?}");
    Ok(rv.or(u_return))
}

fn print_row(label: &str, row: Result<(u32, Vec<u8>), String>) {
    match row {
        Ok((list, statuses)) => {
            println!("{label}: list={list} status=[{}]", statuses.iter().map(|b| b.to_string()).collect::<Vec<_>>().join(","))
        }
        Err(e) => println!("{label}: ERROR {e}"),
    }
}

fn main() {
    println!("probe_mi: native MI transport (ticket 16)");
    let connection = match MiConnection::connect() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("connect failed: {e}");
            std::process::exit(1);
        }
    };
    println!("connected (MI client + local session)");

    println!("== BatteryControl readback row (battery 1, query 1) ==");
    print_row("row", read_row(&connection, 1, 1));

    println!("== SetBatteryHealthControl tuples (status=1 enable) ==");
    for (battery, mask) in [(1u8, 1u8), (1u8, 2u8), (1u8, 3u8)] {
        println!("  set ({battery},{mask},1):");
        let rv = set_tuple(&connection, battery, mask, 1);
        println!("    attempt result: {rv:?}");
        thread::sleep(Duration::from_millis(400));
        print_row("  row after", read_row(&connection, 1, 1));
    }

    println!("== SetBatteryHealthControl (1,2,0) disable, then restore (1,2,1) ==");
    println!("  set (1,2,0):");
    let _ = set_tuple(&connection, 1, 2, 0);
    thread::sleep(Duration::from_millis(400));
    print_row("  row after", read_row(&connection, 1, 1));
    println!("  set (1,2,1) restore:");
    let _ = set_tuple(&connection, 1, 2, 1);
    thread::sleep(Duration::from_millis(400));
    print_row("  row after", read_row(&connection, 1, 1));

    println!("== AcerGamingFunction: profile readback + fan ==");
    match connection.enumerate_first_instance(NAMESPACE, "AcerGamingFunction") {
        Ok(instance) => {
            println!("    instance class: {}", instance.class_name());
            let mut input = connection.new_instance("AcerGamingFunction").expect("new instance");
            input.add_u32("gmInput", 0x0B).expect("add gmInput");
            match connection.invoke(NAMESPACE, &instance, "GetGamingMiscSetting", &input) {
                Ok(Some(result)) => {
                    let gm = result.get_u64("gmOutput").ok().flatten().unwrap_or(0);
                    println!("    GetGamingMiscSetting(0x0B): gmOutput=0x{gm:X} (profile {})", gm >> 8);
                }
                Ok(None) => println!("    GetGamingMiscSetting: no out-params"),
                Err(e) => println!("    GetGamingMiscSetting: ERROR {e}"),
            }
            let mut fan = connection.new_instance("AcerGamingFunction").expect("new instance");
            fan.add_u32("gmInput", 0).expect("add gmInput");
            match connection.invoke(NAMESPACE, &instance, "GetGamingFanBehavior", &fan) {
                Ok(Some(result)) => {
                    let gm = result.get_u64("gmOutput").ok().flatten().unwrap_or(0);
                    println!("    GetGamingFanBehavior: gmOutput={gm} (0x{gm:X})");
                }
                Ok(None) => println!("    GetGamingFanBehavior: no out-params"),
                Err(e) => println!("    GetGamingFanBehavior: ERROR {e}"),
            }
        }
        Err(e) => println!("    AcerGamingFunction: ERROR {e}"),
    }
    println!("probe_mi complete");
}
