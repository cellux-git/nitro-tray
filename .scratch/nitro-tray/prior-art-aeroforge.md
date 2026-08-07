# Prior art: AeroForge hardware control encodings (extracted reference tables)

Read-only extraction from `aeroforge-service\src\workers\control\` in the sibling repo
`aeroforge-nitrosense-alternative` (commit state = local working tree, unmodified by this extraction).
Target hardware: Acer Nitro 16S AI (AN16S-61), AMD platform. AeroForge's own device notes
reference "ANV16-41" for the smart-charge direct-trust path.

All values below are runtime-observed interface facts from the AeroForge source. Every item
carries `source: <file>:<line>`.

---

## 1. WMI layer — `acer_wmi.rs` (source prefix: `aeroforge-service\src\workers\control\acer_wmi.rs`)

### 1.1 Connection/namespace/class

| Item | Value | Source |
|---|---|---|
| WMI namespace | `ROOT\WMI` | acer_wmi.rs:47 |
| Class name | `AcerGamingFunction` | acer_wmi.rs:48 |
| Instance object path for ExecMethod | `AcerGamingFunction.InstanceName="ACPI\\PNP0C14\\APGe_0"` | acer_wmi.rs:49-50 |
| In-proc params (WMI method signature) | `gmInput` (in), `gmOutput` (out) | acer_wmi.rs:51-52 |
| COM init | `CoInitializeEx(COINIT_MULTITHREADED)`; `CoInitializeSecurity(RPC_C_AUTHN_LEVEL_DEFAULT, RPC_C_IMP_LEVEL_IMPERSONATE)` | acer_wmi.rs:202-232 |
| Locator | `CoCreateInstance(CLSID_WBEM_LOCATOR = {4590F811-1D3A-11D0-891F-00AA004B2E24}, CLSCTX_INPROC_SERVER, IID_IWBEM_LOCATOR = {DC12A687-737F-11CF-884D-00AA004B2E24})` | acer_wmi.rs:34-45, 510-526 |
| Connect | `IWbemLocator::ConnectServer("ROOT\WMI")` + `CoSetProxyBlanket` | acer_wmi.rs:528-567 |
| Class fetch | `IWbemServices::GetObject("AcerGamingFunction")` | acer_wmi.rs:581-602 |
| Invocation | `GetMethod` -> `SpawnInstance` -> `Put("gmInput")` -> `IWbemServices::ExecMethod(objectPath, method, input)` -> read `gmOutput` | acer_wmi.rs:653-683, 685-742, 604-635, 744-772 |
| gmInput CIM type | CIM_UINT32 first; falls back to CIM_UINT64 as decimal BSTR string | acer_wmi.rs:685-742 |
| gmOutput decode | `u64` variant (VT_UI1..VT_UI8, VT_BSTR decimal parse); `WBEM_E_NOT_FOUND` (0x80041002) => None | acer_wmi.rs:744-772, 866-892, 762 |
| Fallback when COM path fails | hidden PowerShell `Invoke-CimMethod -Namespace root\wmi -ClassName AcerGamingFunction -Arguments @{ gmInput = ... }`; result read from `gmOutput` | acer_wmi.rs:285-347 |

### 1.2 Method table

| Method (on AcerGamingFunction) | Rust wrapper | Input encoding | Source |
|---|---|---|---|
| `SetGamingProfile` | `apply_gaming_profile` | raw `u64` = profile value | acer_wmi.rs:89-93 |
| `SetGamingMiscSetting` | `apply_gaming_misc_setting(setting, value)` | `input = setting \| (value << 8)` | acer_wmi.rs:95-101 (encoding at :99) |
| `GetGamingMiscSetting` | `read_gaming_misc_setting(setting)` | `input = setting` | acer_wmi.rs:103-107 |
| `SetGamingFanBehavior` | `apply_fan_behavior` | raw `u64` = behavior input | acer_wmi.rs:109-113 |
| `SetGamingFanSpeed` | `apply_fan_speed(selector, percent)` | `input = (clamped_percent << 8) \| selector` | acer_wmi.rs:115-125 (`fan_speed_input` :122-125) |
| `GetGamingSysInfo` | `read_gaming_sys_info(input)` | `input = u32 flag` | acer_wmi.rs:172-176 |

### 1.3 Profile values

| Profile | Value | Source |
|---|---|---|
| Quiet | `0x00` (u8) | acer_wmi.rs:59 |
| Balanced | `0x0000_0001` | acer_wmi.rs:56 |
| Performance | `0x0000_0004` | acer_wmi.rs:57 |
| Turbo | `0x0000_0005` | acer_wmi.rs:58 |
| Eco | `0x0000_0006` (bit 6 only — see 1.6; NO apply constant exists) | power.rs:523 |

### 1.4 Misc-setting flags (setting byte)

| Flag | Value | Used for | Source |
|---|---|---|---|
| `MISC_SETTING_SUPPORTED_PROFILES` | `0x0A` | read-only probe of supported-profile bitmask via `GetGamingMiscSetting` | acer_wmi.rs:69 |
| `MISC_SETTING_PLATFORM_PROFILE` | `0x0B` | set current platform profile via `SetGamingMiscSetting(0x0B, profile)`; read back via `GetGamingMiscSetting(0x0B)` | acer_wmi.rs:70 |

Platform-profile write flow (power.rs): `SetGamingMiscSetting(0x0B, value)` is the primary path;
`SetGamingProfile(value)` is the legacy fallback. gmOutput accepted when `None | Some(0) | Some(1)`
or `decode_gm_output_byte(output) == expected`. Source: power.rs:318-387, 473-482.

### 1.5 Fan behavior values (SetGamingFanBehavior input)

| Behavior | Value | Source |
|---|---|---|
| `FAN_BEHAVIOR_AUTO` | `0x0041_0009` | acer_wmi.rs:61 |
| `FAN_BEHAVIOR_MAX` | `0x0082_0009` | acer_wmi.rs:62 |
| `FAN_BEHAVIOR_CUSTOM_MIXED` | `0x00C3_0009` | acer_wmi.rs:63 |

Fan selectors: CPU = `0x01`, GPU = `0x04` (acer_wmi.rs:65-66); `MIN_MANUAL_FAN_PERCENT = 2`
(acer_wmi.rs:67); percent 0 stays 0, else clamped to 2..=100 (acer_wmi.rs:72-78).

Profile -> behavior mapping (fan.rs): Auto -> `0x00410009`, Max -> `0x00820009`,
Custom -> `0x00C30009`. Source: fan.rs:783-789.

### 1.6 GetGamingSysInfo readback flags (u32 input)

| Flag | Sensor | Decode | Source |
|---|---|---|---|
| `0x0000` | supported-sensor bitmask | `(value >> 24) & 0xFFFF` | acer_wmi.rs:134, 162, 468 |
| `0x0002` | battery status | raw u64 | acer_wmi.rs:135 |
| `0x0101` | CPU temp °C | `(value >> 8) & 0xFFFF` (`decode_sysinfo_sensor`) | acer_wmi.rs:136, 164, 478-480 |
| `0x0201` | CPU fan RPM | `(value >> 8) & 0xFFFF` | acer_wmi.rs:137, 167 |
| `0x0301` | system temp °C | `(value >> 8) & 0xFFFF` | acer_wmi.rs:138, 166 |
| `0x0601` | GPU fan RPM | `(value >> 8) & 0xFFFF` | acer_wmi.rs:139, 168 |
| `0x0A01` | GPU temp °C | `(value >> 8) & 0xFFFF` | acer_wmi.rs:140, 165 |

No profile/fan-mode readback flag exists on GetGamingSysInfo. Profile readback is done via
`GetGamingMiscSetting(0x0B)`; the supported-profile bitmask via `GetGamingMiscSetting(0x0A)`
(bit0=quiet, bit1=balanced, bit4=performance, bit5=turbo, bit6=eco — power.rs:516-533).
`GetGamingFanBehavior` is never called anywhere in the service (no fan-mode readback).

AMD gmOutput byte decode (`decode_gm_output_byte`, used for readback comparisons):
`(value >> 8) & 0xFF` when that byte is nonzero OR value > 0xFF, else low byte. acer_wmi.rs:80-87.

### 1.7 Encoding unit tests (acer_wmi.rs:1081-1118)

| Test | Assertions | Source |
|---|---|---|
| `decodes_amd_shifted_gm_output_bytes` | `decode_gm_output_byte(0x73_00)==0x73`, `(0x01_00)==0x01`, `(0x04_00)==0x04`, `(0x05_00)==0x05` | acer_wmi.rs:1088-1094 |
| `keeps_legacy_low_byte_gm_outputs` | `decode_gm_output_byte(0x00)==0x00`, `(0x01)==0x01`, `(0x64)==0x64` | acer_wmi.rs:1096-1101 |
| `uses_confirmed_acer_fan_behavior_inputs` | AUTO==0x00410009, MAX==0x00820009, CUSTOM_MIXED==0x00C30009 | acer_wmi.rs:1103-1108 |
| `encodes_manual_fan_speed_as_percent_byte_plus_fan_index` | `fan_speed_input(CPU,0)==0x0001`; `(GPU,0)==0x0004`; `(CPU,20)==0x1401`; `(GPU,80)==0x5004`; `(CPU,100)==0x6401`; `(GPU,100)==0x6404` | acer_wmi.rs:1110-1118 |

---

## 2. HID layer — `acer_hid.rs` (+ `telemetry\acer_hid_status.rs`)

### 2.1 Device discovery

| Item | Value | Source |
|---|---|---|
| Vendor ID filter | `ACER_VENDOR_ID = 0x1025` | acer_hid.rs:30 |
| Device path marker | `hid#1025174b&col01#` (device path must contain, lowercase) | acer_hid.rs:31 |
| Enumeration | `SetupDiGetClassDevsW(HidD_GetHidGuid(), DIGCF_PRESENT \| DIGCF_DEVICEINTERFACE)` + `SetupDiEnumDeviceInterfaces` + path marker match + `HidD_GetAttributes` VendorID check | acer_hid.rs:158-275 |
| Open | `CreateFileW(GENERIC_READ\|GENERIC_WRITE, FILE_SHARE_READ\|FILE_SHARE_WRITE, OPEN_EXISTING)` | acer_hid.rs:277-301 |
| Write | `HidD_SetFeature(handle, request, 65)` | acer_hid.rs:97-98 |
| Report ID / length | `REPORT_ID = 0xA0`, `REPORT_LEN = 65` (request buffer 65 bytes, bytes 0..8 form the 9-byte prefix) | acer_hid.rs:32-33 |

### 2.2 System-usage-mode report bytes (65-byte feature report; 9-byte prefix, rest zero)

`build_system_usage_mode_request(mode)` prefix: `[A0, 00, A0, 01, 00, 01, <mode>, 00, 00]`
(acer_hid.rs:126-138)

| Mode | `<mode>` byte | Full 9-byte prefix | Source |
|---|---|---|---|
| Turbo | `0x00` | `A0 00 A0 01 00 01 00 00 00` | acer_hid.rs:55, 349-351 |
| Performance | `0x01` | `A0 00 A0 01 00 01 01 00 00` | acer_hid.rs:56 |
| Normal | `0x02` | `A0 00 A0 01 00 01 02 00 00` | acer_hid.rs:57 |
| Quiet | `0x03` | `A0 00 A0 01 00 01 03 00 00` | acer_hid.rs:58, 353-355 |

### 2.3 Turbo OC-profile hint reports (only sent when applying Turbo)

| Report | 9-byte prefix | Source |
|---|---|---|
| app-status (enable) | `A0 00 A0 03 11 01 03 01 00` | acer_hid.rs:140-144, 361-363 |
| oc-profile-select(profile=0) | `A0 00 A0 02 00 01 00 00 00` | acer_hid.rs:146-150, 365-367 |

### 2.4 Eco mapping

No eco mode in the HID layer. `SystemUsageMode` is only Turbo/Performance/Normal/Quiet
(acer_hid.rs:36-41). In the power layer, quiet (BatteryGuard) maps to HID `SystemUsageMode::Quiet`
(power.rs:178); Balanced->Normal, Performance->Performance, Turbo->Turbo (power.rs:271-278).
Eco has no HID write path.

### 2.5 HID readback

- Write path readback: `HidD_GetFeature` with response[0]=0xA0, 65 bytes; logged as hex prefix
  (acer_hid.rs:103-116) — no parsed value.
- Telemetry status readback (`telemetry\acer_hid_status.rs`): request `[0]=0xA0, [2]=0xA0,
  [3]=0x08 (STATUS_GROUP), [5]=0x02, [6]=<selector>`, rest zero, 65 bytes; response value =
  `u16::from_le_bytes([response[8], response[9]])`. Selectors: `1`=CPU temp °C, `2`=CPU fan RPM,
  `3`=system temp °C, `6`=GPU fan RPM. Source: acer_hid_status.rs:35-36, 210-246, 60-65.
  Same device marker `hid#1025174b&col01#` + VID 0x1025 (acer_hid_status.rs:33-34).

---

## 3. Smart charge — `smart_charge.rs`

### 3.1 WMI surface

| Item | Value | Source |
|---|---|---|
| Namespace/class | `root\wmi` / `BatteryControl` (Get-CimClass + Get-CimInstance; first instance) | smart_charge.rs:134, 270 |
| Write methods | `SetBatteryHealthControl(uBatteryNo, uFunctionMask, uFunctionStatus, uReservedIn[5])`; `SetBatteryFunctionData(uBACSwitch, uFunctionMask, uReservedIn[5])` | smart_charge.rs:281-286, 440-443, 455, 491 |
| Read methods | `GetBatteryHealthControlStatus(uBatteryNo, uFunctionQuery, uReserved[2])` -> `uFunctionList`, `uFunctionStatus[]`, `uReturn[]`; `GetBatteryFunctionData(uFunctionMask, uReservedIn[5])` -> `uBACStatus`, `uBACStartTime`, `uBACStopTime`, `uReturnCode`, `uReservedOut` | smart_charge.rs:186-197, 210-221, 298-304, 357-364 |
| Request encoding (on/off) | `requested_health_status = 1` (enabled / 80% ceiling active) or `0` (disabled / full charge) | smart_charge.rs:38 |

### 3.2 The AMD direct-trust write path (THE CRITICAL ITEM)

Same `BatteryControl` class/methods as everything else — the difference is the **argument tuple**:
battery **1**, mask **1**, scalar status byte, 5-zero reserved. It is attempted FIRST, before the
generic battery-0/1 sweep, and is model-specific ("ANV16-41"):

```powershell
# made by faxcon
# ANV16-41 / uBatteryNo=1, uFunctionMask=1, 5-byte reserved - trust set only.
$setAnv = Invoke-CimMethod -InputObject $battery -MethodName SetBatteryHealthControl -Arguments @{
    uBatteryNo = [byte]1
    uFunctionMask = [byte]1
    uFunctionStatus = $status
    uReservedIn = [byte[]](0,0,0,0,0)
} -ErrorAction Stop
if ($setAnv.ReturnValue) {
    Emit-AeroForgeResult ([ordered]@{
        requestedHealthStatus = [int]$status
        healthStatus = [int]$status
        setAttempt = 'battery1-health-byte0-anv16x41-direct'
    })
}
```

Source: smart_charge.rs:279-293. The result is emitted immediately without readback verification
when the set returns nonzero `ReturnValue`.

### 3.3 Fallback attempts (if direct-trust set fails)

| Attempt name | Method | Args | Source |
|---|---|---|---|
| `battery1-health-byte1-scalar` / `battery0-...` | SetBatteryHealthControl | uBatteryNo=1(or 0), uFunctionMask=`2`, uFunctionStatus=$status, uReservedIn=[0,0,0,0,0] | smart_charge.rs:404-412, 449-450 |
| `battery1-combined-byte0-byte1-scalar` / `battery0-...` | SetBatteryHealthControl | uBatteryNo, uFunctionMask=`3`, uFunctionStatus=$status, uReservedIn=[0,0,0,0,0] | smart_charge.rs:413-421 |
| `battery1-legacy-byte0-scalar` / `battery0-...` | SetBatteryHealthControl | uBatteryNo, uFunctionMask=`1`, uFunctionStatus=$status, uReservedIn=[0,0,0,0,0] | smart_charge.rs:422-430 |
| `battery-function-data-mask{2,1,3,0,4,5,7}` | SetBatteryFunctionData | uBACSwitch=$status, uFunctionMask in [2,1,3,0,4,5,7], uReservedIn=[0,0,0,0,0] | smart_charge.rs:433-446, 487-488 |

Each SetBatteryHealthControl attempt is followed by 250 ms sleep + readback match
(`Find-DesiredStatus`); each SetBatteryFunctionData attempt by 350 ms + `Find-DesiredFunctionData`.
Sources: smart_charge.rs:456-457, 491-493.

### 3.4 Readback matching

- `Find-DesiredStatus`: reads `GetBatteryHealthControlStatus` for uBatteryNo in 0..4 and
  uFunctionQuery in 0..6; prefers a row where `(uFunctionList & 2) != 0` and `uFunctionStatus[1] ==
  requested`; else any row where `uFunctionStatus[i] == requested`. Last-resort health = max
  non-255 status byte. Sources: smart_charge.rs:306-355.
- `Find-DesiredFunctionData`: reads `GetBatteryFunctionData` for uFunctionMask in [0,1,2,3,4,5,7,255];
  match on `uBACStatus == requested`. Sources: smart_charge.rs:366-400.
- Result is validated: `parsed.health_status == requested_health_status` or error.
  smart_charge.rs:566-572.
- PS output parsed from lines prefixed `AEROFORGE_BATTERY_CONTROL_RESULT:`; unit test asserts
  non-prefixed JSON lines are ignored (smart_charge.rs:12, 577-596, 608-619).

---

## 4. Power layer — `power.rs` (+ `control.rs` defaults)

### 4.1 Powercfg scheme/alias constants

| Constant | Value | Source |
|---|---|---|
| `SUB_PROCESSOR` | `SUB_PROCESSOR` (powercfg alias) | power.rs:25 |
| `PROCTHROTTLEMIN` | `PROCTHROTTLEMIN` (alias of GUID_PROCESSOR_THROTTLE_MINIMUM, used by alias not GUID) | power.rs:26 |
| `PROCTHROTTLEMAX` | `PROCTHROTTLEMAX` (alias of GUID_PROCESSOR_THROTTLE_MAXIMUM, used by alias not GUID) | power.rs:27 |
| `SCHEME_CURRENT` | `SCHEME_CURRENT` (alias of active scheme GUID) | power.rs:28 |

No boost-mode GUIDs, no GUID_PROCESSOR_BOOST_MODE / THROTTLE_MINIMUM / THROTTLE_MAXIMUM constants
exist in the source. No power-plan duplication or renaming exists (no `PowerDuplicateScheme`).
All writes target the **current active scheme** only.

### 4.2 Commands issued

| Command | Purpose | Source |
|---|---|---|
| `powercfg /setacvalueindex SCHEME_CURRENT SUB_PROCESSOR PROCTHROTTLEMIN <pct>` | AC min % | power.rs:65-69, 596-608 |
| `powercfg /setacvalueindex SCHEME_CURRENT SUB_PROCESSOR PROCTHROTTLEMAX <pct>` | AC max % | power.rs:70-74 |
| `powercfg /setdcvalueindex SCHEME_CURRENT SUB_PROCESSOR PROCTHROTTLEMIN <pct>` | DC min % | power.rs:75-79 |
| `powercfg /setdcvalueindex SCHEME_CURRENT SUB_PROCESSOR PROCTHROTTLEMAX <pct>` | DC max % | power.rs:80-84 |
| `powercfg /setactive SCHEME_CURRENT` | re-apply active scheme after writes | power.rs:85 |
| `powercfg /q SCHEME_CURRENT SUB_PROCESSOR <setting>` | readback | power.rs:671 |

Readback parsing: look for "Current AC Power Setting Index"/"Current DC Power Setting Index"
labeled hex (`parse_labeled_powercfg_value`, power.rs:701-723) or fall back to the last two hex
values in the scoped block, AC = second-to-last, DC = last (`parse_scoped_powercfg_value`,
power.rs:725-779). Sanitization clamps min to 0..100, max to 5..100, min<=max (power.rs:145-163).

### 4.3 Default processor-state table (per profile) — `control.rs`

| Profile | CPU min % | CPU max % | Source |
|---|---|---|---|
| BatteryGuard (quiet) | 5 | 45 | control.rs:698-701 |
| Balanced | 35 | 88 | control.rs:702-705 |
| Performance | 100 | 100 | control.rs:706-711 |
| Turbo | 100 | 100 | control.rs:706-711 |
| Custom | 35 | 88 | control.rs:712-715 |

### 4.4 Profile -> firmware + HID mapping (power.rs)

| PowerProfileId | WMI profile value | SetGamingMiscSetting(0x0B, v) | HID SystemUsageMode | Source |
|---|---|---|---|---|
| BatteryGuard | `0x00` (quiet) | yes (primary) | Quiet | power.rs:171-188, 271-278 |
| Balanced | `0x0001` | yes (primary) | Normal | power.rs:189-195, 271-278 |
| Performance | `0x0004` | yes (primary) | Performance | power.rs:196-202, 271-278 |
| Turbo | `0x0005` | yes (primary) | Turbo (+ OC-profile hint) | power.rs:203-209, 260-264, 271-278 |
| Custom | base: Balanced `0x0001` / Performance `0x0004` / Turbo `0x0005` (default Performance) | yes (primary) | per base | power.rs:210-216, 561-567 |

Fallback: `SetGamingProfile(value)` if the misc-setting path errors or is rejected (gmOutput not
in {None, 0, 1} and decoded byte != target). Sources: power.rs:318-387, 389-471.

### 4.5 Power layer unit tests (power.rs:794-829)

| Test | Assertions | Source |
|---|---|---|
| `parses_localized_processor_state_indexes` | PT-PT powercfg output: PROCTHROTTLEMIN AC=35 (`0x00000023`), DC=45 (`0x0000002D`) via scoped fallback | power.rs:798-819 |
| `keeps_english_label_fast_path` | "Current AC Power Setting Index: 0x00000058" -> 88 | power.rs:821-829 |

---

## 5. Models & state — `models.rs`, `state.rs`

### 5.1 PowerProfileId enum

`BatteryGuard, Balanced, Performance (alias "performance"), Turbo, Custom` — kebab-case.
**No Eco variant.** Source: models.rs:4-13.

### 5.2 FanProfileId enum

`Auto, Max, Custom` with labels "auto"/"max"/"custom". Source: models.rs:47-63.

### 5.3 Smart-charge request/snapshot

`ApplySmartChargeRequest { enabled: bool }` (models.rs:118-122); snapshot carries `enabled`,
`health_status` (u8), `battery_healthy` (0 when health_status==1, else 1) (models.rs:273-281;
smart_charge.rs:48).

### 5.4 state.rs

Pure snapshot persistence (JSON, atomic tmp+MoveFileExW replace, BOM-tolerant load). No WMI
encodings. Sources: state.rs:196-266.

---

## 6. Licensing & branding

| Item | Value | Source |
|---|---|---|
| LICENSE file | **None in repo root.** Only third-party vendored licenses exist: `aeroforge-service\vendor\winring0\LICENSE.txt`, `third_party\pawnio-modules\COPYING` | repo scan |
| package.json license field | absent (`"private": true`, name `aeroforge-control`, version `0.16.3`) | package.json:1-4 |
| src-tauri Cargo.toml | `license = ""` | src-tauri/Cargo.toml:6 |
| App brand | "AeroForge Control" (portable folder `AeroForge Control Portable`, debug bundle `AeroForge-Debug-Collector.cmd`) | README.md:67, 84-99 |
| Official Acer app | "NitroSense" referenced: `c:\program files\nitrosense\nitrosense.exe` + `nitrosenselauncher.exe` (AeroForge kills these via a 30s "nitro guard" sweep) | src-tauri\src\backend\nitro_guard.rs:5-9 |
| Clean-room note | AeroForge treats vendor names/WMI class/method names/numeric inputs as runtime-observed interface facts, not Acer-derived code | README.md:173-176 |

---

## 7. Not found in the code (gaps for nitro-tray implementers)

1. **Eco profile apply path** — no `GAMING_PROFILE_ECO` constant, no PowerProfileId::Eco, no eco
   HID mode. Eco appears only as bit 6 in the supported-profiles bitmask label list
   (power.rs:523). If NitroSense-style ECO is required, the `0x00000006` misc-setting value is
   implied but unproven in this codebase.
2. **GetGamingFanBehavior readback** — never invoked; no fan-mode readback exists.
3. **Fan-mode readback via GetGamingSysInfo** — flags only cover temps/RPM/battery/support mask.
4. **Power-plan management** — no PowerDuplicateScheme/plan rename, no "Nitro-Quiet/-Balanced/
   -Performance/-Eco" plan names anywhere; only SCHEME_CURRENT writes.
5. **Boost-mode GUIDs** — no GUID_PROCESSOR_BOOST_MODE/disabled/enabled/aggressive values;
   boost mode is not touched by AeroForge.
6. **Explicit GUID_PROCESSOR_THROTTLE_MINIMUM/MAXIMUM** — AeroForge uses the powercfg aliases
   PROCTHROTTLEMIN/PROCTHROTTLEMAX, not the GUIDs.
7. **License** — the AeroForge repo publishes no license (no LICENSE file, private package.json,
   empty Cargo.toml license field); treat prior-art status as unlicensed/unstated.
