//! Elevated diagnostic probe: shape-matrix test of the WBEM-COM input
//! layer against the Acer WMI classes (ticket 15 investigation).
//!
//! Test-time only — run ON the target laptop from an elevated shell.
//!
//! The app's put chain tries VT_UI8/CIM_UINT64, VT_UI4/CIM_UINT32, then a
//! decimal BSTR; on this machine all three have been rejected with
//! WBEM_E_TYPE_MISMATCH while the MI stack (`Invoke-CimMethod`) accepts the
//! same values. This probe sweeps every plausible (VARIANT, CIMTYPE) pair on
//! `SetGamingMiscSetting` and, for any pair that survives `Put`, tries
//! `ExecMethod` with the class name as the target path (no instance lookup),
//! the enumerated first instance, and the hardcoded Acer instance path. It
//! also probes `BatteryControl` `GetBatteryHealthControlStatus` the same way
//! (read-only method — no hardware state is changed by this probe).

use nitro_tray::comwbem::{
    self, Bstr, ClassObject, ComApartment, ComRef, Variant, CIM_SINT32, CIM_SINT64, CIM_UINT32,
    CIM_UINT64, CIM_UINT8,
};

const NAMESPACE: &str = "ROOT\\WMI";
const ACER_INSTANCE: &str = "AcerGamingFunction.InstanceName=\"ACPI\\PNP0C14\\APGe_0\"";

/// One (variant, cimtype) put attempt.
struct Shape {
    label: &'static str,
    variant: Variant,
    cimtype: i32,
}

fn hr_name(hr: i32) -> String {
    let code = ((hr as u32) & 0xFFFF) as u16;
    let name = match code {
        0x1001 => "WBEM_E_FAILED",
        0x1002 => "WBEM_E_NOT_FOUND",
        0x1003 => "WBEM_E_ACCESS_DENIED",
        0x1004 => "WBEM_E_PROVIDER_LOAD_FAILURE",
        0x1005 => "WBEM_E_TYPE_MISMATCH",
        0x1006 => "WBEM_E_OUT_OF_MEMORY",
        0x1008 => "WBEM_E_INVALID_PARAMETER",
        0x1009 => "WBEM_E_NOT_AVAILABLE",
        0x100A => "WBEM_E_CRITICAL_ERROR",
        0x100E => "WBEM_E_INVALID_NAMESPACE",
        0x1010 => "WBEM_E_INVALID_CLASS",
        0x1011 => "WBEM_E_CALL_CANCELLED",
        0x1016 => "WBEM_E_INVALID_OPERATION",
        0x1017 => "WBEM_E_INVALID_QUERY",
        0x102E => "WBEM_E_PROPAGATED_QUALIFIER",
        0x103A => "WBEM_E_PROVIDER_NOT_FOUND",
        _ => "?",
    };
    format!("0x{code:04X} {name}")
}

fn report(label: &str, hr: i32) {
    println!("    {label}: {}", hr_name(hr));
}

/// Build the gmInput put matrix (shapes the provider may accept).
fn gm_input_shapes(value: u64) -> Vec<Shape> {
    let text = value.to_string();
    let bstr = Bstr::new(&text).expect("bstr");
    vec![
        Shape { label: "VT_UI8 + CIM_UINT64 (declared)", variant: Variant::ui8(value), cimtype: CIM_UINT64 },
        Shape { label: "VT_UI4 + CIM_UINT32 (declared)", variant: Variant::ui4(value as u32), cimtype: CIM_UINT32 },
        Shape { label: "VT_I4 + CIM_SINT32", variant: Variant::i4(value as i32), cimtype: CIM_SINT32 },
        Shape { label: "VT_I8 + CIM_SINT64", variant: Variant::i8(value as i64), cimtype: CIM_SINT64 },
        Shape { label: "VT_UI8 + CIMTYPE 0", variant: Variant::ui8(value), cimtype: 0 },
        Shape { label: "VT_UI4 + CIMTYPE 0", variant: Variant::ui4(value as u32), cimtype: 0 },
        Shape { label: "VT_I4 + CIMTYPE 0", variant: Variant::i4(value as i32), cimtype: 0 },
        Shape { label: "VT_UI8 + CIM_UINT32 (crossed)", variant: Variant::ui8(value), cimtype: CIM_UINT32 },
        Shape { label: "VT_UI4 + CIM_UINT64 (crossed)", variant: Variant::ui4(value as u32), cimtype: CIM_UINT64 },
        Shape { label: "BSTR + CIM_UINT64", variant: Variant::from_bstr(bstr.into_raw()), cimtype: CIM_UINT64 },
    ]
}

/// Put matrix over the input instance; returns the first hr == 0 shape label.
fn put_matrix(in_params: &ClassObject, name: &str, shapes: Vec<Shape>) -> Option<(String, i32)> {
    let mut winner: Option<(String, i32)> = None;
    for shape in shapes {
        let hr = unsafe { in_params.put(comwbem::wide(name).as_ptr(), &shape.variant, shape.cimtype) };
        if hr == 0 {
            winner = Some((shape.label.to_string(), hr));
            println!("    PUT OK: {}", shape.label);
        } else {
            println!("    put {}: {}", shape.label, hr_name(hr));
        }
    }
    winner
}

/// Spawn the input instance for `method` and put the scalar under `name`.
fn spawn_and_put(
    class: &ClassObject,
    method: &str,
    name: &str,
    shapes: Vec<Shape>,
) -> Option<(ClassObject, String)> {
    let method_wide = comwbem::wide(method);
    let mut in_class: *mut core::ffi::c_void = core::ptr::null_mut();
    let mut out_class: *mut core::ffi::c_void = core::ptr::null_mut();
    let hr = unsafe { class.get_method(method_wide.as_ptr(), &mut in_class, &mut out_class) };
    if hr != 0 {
        report(&format!("GetMethod({method})"), hr);
        return None;
    }
    println!("    GetMethod({method}): ok");
    drop(ComRef::from_raw(out_class));
    let in_class = unsafe { ClassObject::from_raw(in_class) };
    let mut in_params: *mut core::ffi::c_void = core::ptr::null_mut();
    let hr = unsafe { in_class.spawn_instance(&mut in_params) };
    if hr != 0 {
        report("SpawnInstance", hr);
        return None;
    }
    let in_params = unsafe { ClassObject::from_raw(in_params) };
    put_matrix(&in_params, name, shapes).map(|(label, _)| (in_params, label))
}

/// ExecMethod with the given target path; prints the outcome.
fn try_exec(
    services: &ComRef,
    path: &Bstr,
    method: &str,
    in_params: &ClassObject,
) -> Result<Option<u64>, i32> {
    let method_bstr = Bstr::new(method).expect("method bstr");
    let out = unsafe { comwbem::exec_method(services, path, &method_bstr, in_params.raw()) }?;
    match out {
        None => Ok(None),
        Some(out) => Ok(unsafe { out.get(comwbem::wide("gmOutput").as_ptr()) }
            .ok()
            .and_then(|v| v.as_u64())),
    }
}

fn main() {
    let _com = ComApartment::init().expect("com init");
    println!("probe: WBEM-COM put-shape matrix (ticket 15)");
    let locator = unsafe { comwbem::create_locator() }.expect("locator");
    let services = unsafe { comwbem::connect_server(&locator, NAMESPACE) }.expect("connect ROOT\\WMI");
    println!("connected to {NAMESPACE}");

    // --- AcerGamingFunction: shape matrix per method ---
    println!("== AcerGamingFunction shape matrix ==");
    let class = match unsafe { comwbem::get_class(&services, "AcerGamingFunction") } {
        Ok(class) => class,
        Err(hr) => {
            report("GetObject(AcerGamingFunction)", hr);
            return;
        }
    };
    println!("    GetObject(AcerGamingFunction): ok");
    let instance_path = match unsafe { comwbem::first_instance_path(&services, "AcerGamingFunction") } {
        Ok(path) => path,
        Err(hr) => {
            report("first_instance_path(AcerGamingFunction)", hr);
            return;
        }
    };
    println!("    first_instance_path: ok");

    let read_value = 0x0Bu64; // GetGamingMiscSetting(0x0B): current platform profile
    let set_value = 0x40Bu64; // SetGamingMiscSetting(0x0B, 4): performance — idempotent no-op if already 4
    let cases = [
        ("GetGamingMiscSetting", read_value),
        ("SetGamingMiscSetting", set_value),
        ("GetGamingFanBehavior", 0u64),
        ("SetGamingFanBehavior", 0x0041_0009u64), // fan auto — the intended state
    ];
    for (method, value) in cases {
        println!("  --- {method} ---");
        let shapes = gm_input_shapes(value);
        let Some((in_params, winner)) = spawn_and_put(&class, method, "gmInput", shapes) else {
            println!("    no put shape survived");
            continue;
        };
        println!("    put winner: {winner}");
        match try_exec(&services, &instance_path, method, &in_params) {
            Ok(out) => println!("    ExecMethod(first_instance): ok gmOutput={out:?}"),
            Err(hr) => report("    ExecMethod(first_instance)", hr),
        }
        let path_bstr = Bstr::new(ACER_INSTANCE).expect("path bstr");
        match try_exec(&services, &path_bstr, method, &in_params) {
            Ok(out) => println!("    ExecMethod(hardcoded path): ok gmOutput={out:?}"),
            Err(hr) => report("    ExecMethod(hardcoded path)", hr),
        }
    }

    // --- BatteryControl.GetBatteryHealthControlStatus (read-only) ---
    println!("== BatteryControl.GetBatteryHealthControlStatus ==");
    let class = match unsafe { comwbem::get_class(&services, "BatteryControl") } {
        Ok(class) => class,
        Err(hr) => {
            report("GetObject(BatteryControl)", hr);
            return;
        }
    };
    println!("    GetObject(BatteryControl): ok");
    let ui1_shapes = vec![
        Shape { label: "VT_UI1 + CIM_UINT8 (declared)", variant: Variant::ui1(1), cimtype: CIM_UINT8 },
        Shape { label: "VT_UI1 + CIMTYPE 0", variant: Variant::ui1(1), cimtype: 0 },
        Shape { label: "VT_UI4 + CIM_UINT32", variant: Variant::ui4(1), cimtype: CIM_UINT32 },
    ];
    let method_wide = comwbem::wide("GetBatteryHealthControlStatus");
    let mut in_class: *mut core::ffi::c_void = core::ptr::null_mut();
    let mut out_class: *mut core::ffi::c_void = core::ptr::null_mut();
    let hr = unsafe { class.get_method(method_wide.as_ptr(), &mut in_class, &mut out_class) };
    if hr != 0 {
        report("GetMethod(GetBatteryHealthControlStatus)", hr);
        return;
    }
    drop(ComRef::from_raw(out_class));
    let in_class = unsafe { ClassObject::from_raw(in_class) };
    let mut in_params: *mut core::ffi::c_void = core::ptr::null_mut();
    let hr = unsafe { in_class.spawn_instance(&mut in_params) };
    if hr != 0 {
        report("SpawnInstance", hr);
        return;
    }
    let in_params = unsafe { ClassObject::from_raw(in_params) };
    let Some(_) = put_matrix(&in_params, "uBatteryNo", ui1_shapes) else {
        println!("  no scalar shape survived on BatteryControl");
        return;
    };
    let _ = put_matrix(
        &in_params,
        "uFunctionQuery",
        vec![Shape { label: "VT_UI1 + CIM_UINT8", variant: Variant::ui1(1), cimtype: CIM_UINT8 }],
    );
    match unsafe { comwbem::first_instance_path(&services, "BatteryControl") } {
        Ok(path) => {
            let method_bstr = Bstr::new("GetBatteryHealthControlStatus").expect("method bstr");
            match unsafe { comwbem::exec_method(&services, &path, &method_bstr, in_params.raw()) } {
                Ok(_) => println!("  ExecMethod(first_instance): ok"),
                Err(hr) => report("  ExecMethod(first_instance)", hr),
            }
        }
        Err(hr) => report("first_instance_path(BatteryControl)", hr),
    }
    println!("probe complete");
}
