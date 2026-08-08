//! The transport seam: adapters (WMI, smart charge) depend on a small
//! owned-value interface instead of building `MiConnection` themselves
//! (ticket 01, `.scratch/refactorings/issues/01-seam-behind-firmware-transports.md`).
//! `MiConnection` satisfies the seam in production; a scripted fake satisfies
//! it in tests, so the circuit breaker, the smart-charge write-verify gate
//! and the eco-detection protocol run under `cargo test` without hardware.
//!
//! The interface speaks OWNED VALUE TYPES (`MiInput`/`MiOutput`), never
//! `MiInstance` — a fake cannot fabricate the FFI wrapper (private raw
//! pointer, only `unsafe from_raw`). Typing is explicit per element
//! (`MiValue::U64` vs `MiValue::U32`), replacing the Set/Get method-name
//! dispatch heuristic (the ticket-16 bug class).
//!
//! `MiConnection`'s raw API (`enumerate_first_instance`/`invoke`/
//! `new_instance`/`MiInstance`) stays public — the probes use it directly.

use crate::mi::{
    MiError, MI_RESULT_TYPE_MISMATCH,
};

#[cfg(windows)]
use crate::mi::{MiConnection, MiInstance};

/// One input element of a method call: a parameter name plus its explicitly
/// typed value (the type is part of the value — no method-name inference).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MiElement {
    pub name: &'static str,
    pub value: MiValue,
}

/// An owned value carried across the transport seam. The variants mirror the
/// CIM scalar types the Acer methods declare (u8 scalars, u32, u64, u8
/// arrays) — each variant IS its type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MiValue {
    /// CIM `UInt8` scalar.
    U8(u8),
    /// CIM `UInt32` scalar.
    U32(u32),
    /// CIM `UInt64` scalar.
    U64(u64),
    /// CIM `UInt8Array`.
    U8Array(Vec<u8>),
}

/// The in-params bag of one instance-bound MI invocation: the class the
/// dynamic input instance is created with, plus the typed elements (in
/// insertion order — the wire order of the parameter bag).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MiInput {
    pub class: String,
    pub elements: Vec<MiElement>,
}

impl MiInput {
    /// A new empty input bag for `class` (element names must match the
    /// method's parameter names; the real transport creates the dynamic
    /// instance with `MI_Application_NewInstance`).
    pub fn new(class: &str) -> Self {
        Self { class: class.to_string(), elements: Vec::new() }
    }

    /// Append a `u8` parameter.
    pub fn u8(mut self, name: &'static str, value: u8) -> Self {
        self.elements.push(MiElement { name, value: MiValue::U8(value) });
        self
    }

    /// Append a `u32` parameter.
    pub fn u32(mut self, name: &'static str, value: u32) -> Self {
        self.elements.push(MiElement { name, value: MiValue::U32(value) });
        self
    }

    /// Append a `u64` parameter.
    pub fn u64(mut self, name: &'static str, value: u64) -> Self {
        self.elements.push(MiElement { name, value: MiValue::U64(value) });
        self
    }

    /// Append a `u8` array parameter (CIM `UInt8Array`; the real transport
    /// copies the bytes into the MI instance).
    pub fn u8_array(mut self, name: &'static str, values: Vec<u8>) -> Self {
        self.elements.push(MiElement { name, value: MiValue::U8Array(values) });
        self
    }
}

/// The out-params instance of one invocation, as owned values: only the
/// elements the adapters read are materialized (absent ones are skipped).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MiOutput {
    pub elements: Vec<MiElement>,
}

impl MiOutput {
    /// A new empty output bag (for scripted fakes).
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a `u64` element (builder helper for scripted fakes).
    pub fn with_u64(mut self, name: &'static str, value: u64) -> Self {
        self.elements.push(MiElement { name, value: MiValue::U64(value) });
        self
    }

    /// Append a `u32` element (builder helper for scripted fakes).
    pub fn with_u32(mut self, name: &'static str, value: u32) -> Self {
        self.elements.push(MiElement { name, value: MiValue::U32(value) });
        self
    }

    /// Append a `u8` array element (builder helper for scripted fakes).
    pub fn with_u8_array(mut self, name: &'static str, values: Vec<u8>) -> Self {
        self.elements.push(MiElement { name, value: MiValue::U8Array(values) });
        self
    }

    /// Read a `u64` element: `Ok(None)` when absent, `Err` (type mismatch)
    /// when the element exists with a different variant.
    pub fn u64(&self, name: &str) -> Result<Option<u64>, MiError> {
        match self.element(name) {
            None => Ok(None),
            Some(MiValue::U64(value)) => Ok(Some(*value)),
            Some(_) => Err(type_mismatch(name, "u64")),
        }
    }

    /// Read a `u32` element: `Ok(None)` when absent, `Err` (type mismatch)
    /// when the element exists with a different variant.
    pub fn u32(&self, name: &str) -> Result<Option<u32>, MiError> {
        match self.element(name) {
            None => Ok(None),
            Some(MiValue::U32(value)) => Ok(Some(*value)),
            Some(_) => Err(type_mismatch(name, "u32")),
        }
    }

    /// Read a `u8` array element: `Ok(None)` when absent, `Err` (type
    /// mismatch) when the element exists with a different variant.
    pub fn u8_array(&self, name: &str) -> Result<Option<Vec<u8>>, MiError> {
        match self.element(name) {
            None => Ok(None),
            Some(MiValue::U8Array(values)) => Ok(Some(values.clone())),
            Some(_) => Err(type_mismatch(name, "u8 array")),
        }
    }

    /// The element with `name`, if present.
    fn element(&self, name: &str) -> Option<&MiValue> {
        self.elements.iter().find(|e| e.name == name).map(|e| &e.value)
    }
}

fn type_mismatch(name: &str, expected: &str) -> MiError {
    MiError {
        result: MI_RESULT_TYPE_MISMATCH,
        op: "MiOutput",
        message: Some(format!("{name}: expected {expected} element")),
    }
}

/// The MI transport seam: invoke `method` on the first instance of `class`
/// in `namespace` with `input` as the in-params bag. `Ok(None)` = the call
/// succeeded with no out-params instance; `Ok(Some(output))` = the
/// out-params instance, materialized as owned values.
///
/// Adapters are generic over this trait; production wires `MiConnection`,
/// tests wire a scripted fake.
pub trait MiTransport {
    fn invoke_first_instance(
        &self,
        namespace: &str,
        class: &str,
        method: &str,
        input: &MiInput,
    ) -> Result<Option<MiOutput>, MiError>;

    /// Build a fresh transport. Production: `MiConnection::connect()`
    /// (initiates the MI client + local session; reachability is proven by
    /// the first operation). Tests: the scripted fake produces a transport
    /// whose script the test pre-seeded for the reconnect path. `Sized` so
    /// concrete fakes can construct themselves.
    fn connect() -> Result<Self, MiError>
    where
        Self: Sized;
}

/// The out-param elements the adapters read, probed by name with the type
/// each method's MOF declares. The real out instance carries more elements
/// (e.g. the readback's `uBatteryNo`/`uFunctionQuery` echoes); only the
/// consumed ones cross the seam — absent elements are skipped.
#[cfg(windows)]
const OUT_PROBES: &[(&str, OutProbe)] = &[
    ("gmOutput", OutProbe::U64),
    ("ReturnValue", OutProbe::U32),
    ("uReturn", OutProbe::U32),
    ("uFunctionList", OutProbe::U32),
    ("uFunctionStatus", OutProbe::U8Array),
];

#[cfg(windows)]
enum OutProbe {
    U64,
    U32,
    U8Array,
}

#[cfg(windows)]
impl MiTransport for MiConnection {
    /// The production transport: initiate a fresh MI session, then invoke
    /// `method` on the first instance of `class` (the `-InputObject` binding
    /// target — class-level invocation is rejected by the Acer provider,
    /// ticket 16), build the dynamic input bag from `input`, invoke, then
    /// pull the probed out params into an `MiOutput`. The
    /// `MiConnection`/`MiInstance` raw API is unchanged.
    fn connect() -> Result<Self, MiError> {
        MiConnection::connect()
    }

    fn invoke_first_instance(
        &self,
        namespace: &str,
        class: &str,
        method: &str,
        input: &MiInput,
    ) -> Result<Option<MiOutput>, MiError> {
        let instance = self.enumerate_first_instance(namespace, class)?;
        let mut input_bag = self.new_instance(&input.class)?;
        for element in &input.elements {
            match &element.value {
                MiValue::U8(value) => input_bag.add_u8(element.name, *value)?,
                MiValue::U32(value) => input_bag.add_u32(element.name, *value)?,
                MiValue::U64(value) => input_bag.add_u64(element.name, *value)?,
                MiValue::U8Array(values) => input_bag.add_u8_array(element.name, values)?,
            }
        }
        let out = self.invoke(namespace, &instance, method, &input_bag)?;
        match out {
            None => Ok(None),
            Some(result) => Ok(Some(MiOutput { elements: out_elements(&result)? })),
        }
    }
}

/// Materialize the probed out params of an out-params instance into owned
/// elements; an absent element is skipped (`MI_Instance_GetElement` answers
/// `NO_SUCH_PROPERTY`), a present one with an unexpected type is an error.
#[cfg(windows)]
fn out_elements(result: &MiInstance) -> Result<Vec<MiElement>, MiError> {
    let mut elements = Vec::new();
    for (name, probe) in OUT_PROBES {
        match probe {
            OutProbe::U64 => {
                if let Some(value) = result.get_u64(name)? {
                    elements.push(MiElement { name, value: MiValue::U64(value) });
                }
            }
            OutProbe::U32 => {
                if let Some(value) = result.get_u32(name)? {
                    elements.push(MiElement { name, value: MiValue::U32(value) });
                }
            }
            OutProbe::U8Array => {
                if let Some(value) = result.get_u8_array(name)? {
                    elements.push(MiElement { name, value: MiValue::U8Array(value) });
                }
            }
        }
    }
    Ok(elements)
}

#[cfg(test)]
mod tests {
    use crate::mi::MI_RESULT_TYPE_MISMATCH;
    use crate::transport::{MiElement, MiInput, MiOutput, MiValue};

    #[test]
    fn mi_output_u64_reads_a_present_element() {
        let output = MiOutput::new().with_u64("gmOutput", 0x60B);
        assert_eq!(output.u64("gmOutput"), Ok(Some(0x60B)));
    }

    #[test]
    fn mi_output_u64_is_none_for_absent_elements() {
        let output = MiOutput::new();
        assert_eq!(output.u64("gmOutput"), Ok(None));
    }

    #[test]
    fn mi_output_u64_rejects_a_wrong_typed_element() {
        let output = MiOutput::new().with_u32("gmOutput", 7);
        assert_eq!(
            output.u64("gmOutput"),
            Err(crate::mi::MiError {
                result: MI_RESULT_TYPE_MISMATCH,
                op: "MiOutput",
                message: Some("gmOutput: expected u64 element".into()),
            })
        );
    }

    #[test]
    fn mi_output_u32_reads_a_present_element() {
        let output = MiOutput::new().with_u32("ReturnValue", 1);
        assert_eq!(output.u32("ReturnValue"), Ok(Some(1)));
    }

    #[test]
    fn mi_output_u32_rejects_a_wrong_typed_element() {
        let output = MiOutput::new().with_u64("ReturnValue", 1);
        assert_eq!(
            output.u32("ReturnValue"),
            Err(crate::mi::MiError {
                result: MI_RESULT_TYPE_MISMATCH,
                op: "MiOutput",
                message: Some("ReturnValue: expected u32 element".into()),
            })
        );
    }

    #[test]
    fn mi_output_u8_array_reads_a_present_element() {
        let output = MiOutput::new().with_u8_array("uFunctionStatus", vec![1, 0, 0, 0, 0]);
        assert_eq!(output.u8_array("uFunctionStatus"), Ok(Some(vec![1, 0, 0, 0, 0])));
    }

    #[test]
    fn mi_input_builder_preserves_order_and_typing() {
        let input = MiInput::new("BatteryControl")
            .u8("uBatteryNo", 1)
            .u8("uFunctionMask", 1)
            .u8("uFunctionStatus", 1)
            .u8_array("uReservedIn", vec![0; 5]);
        assert_eq!(input.class, "BatteryControl");
        assert_eq!(
            input.elements,
            vec![
                MiElement { name: "uBatteryNo", value: MiValue::U8(1) },
                MiElement { name: "uFunctionMask", value: MiValue::U8(1) },
                MiElement { name: "uFunctionStatus", value: MiValue::U8(1) },
                MiElement { name: "uReservedIn", value: MiValue::U8Array(vec![0; 5]) },
            ]
        );
    }
}
