//! Elevated diagnostic probe for the Acer HID usage-mode device (ticket 06).
//!
//! Test-time only — run ON the target laptop from an elevated shell (the
//! manifest requires administrator). Never shipped or run by the app itself.
//!
//! Flow: open the vendor 0x1025 device, read the current usage mode
//! (best-effort), then for each of Quiet/Normal/Performance write the
//! usage-mode report, read back a raw feature report and print the 65-byte
//! response hex, and finally restore Quiet.

use nitro_tray::hid::HidAdapter;
use nitro_tray::policy::HidMode;

fn main() {
    let adapter = match HidAdapter::open() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("open failed: {e:?}");
            std::process::exit(1);
        }
    };
    println!("device opened");

    println!("current usage mode (best-effort):");
    match adapter.read_usage_mode() {
        Ok(mode) => println!("  {mode:?}"),
        Err(e) => println!("  read failed (expected off-device protocol gap): {e:?}"),
    }

    let modes = [HidMode::Quiet, HidMode::Normal, HidMode::Performance];
    for mode in modes {
        match adapter.set_usage_mode(mode) {
            Ok(()) => println!("set usage mode {mode:?}: ok"),
            Err(e) => {
                println!("set usage mode {mode:?}: FAILED {e:?}");
                continue;
            }
        }
        match adapter.raw_readback() {
            Ok(response) => {
                println!("  {mode:?} raw response prefix hex:");
                println!("  {}", hex_line(&response));
            }
            Err(e) => println!("  {mode:?} raw readback FAILED {e:?}"),
        }
    }

    println!("restoring Quiet");
    match adapter.set_usage_mode(HidMode::Quiet) {
        Ok(()) => println!("restore ok"),
        Err(e) => eprintln!("restore FAILED {e:?}"),
    }
}

fn hex_line(buf: &[u8]) -> String {
    buf.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" ")
}
