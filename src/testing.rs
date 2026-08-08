//! Test-support fakes (compiled only under `cfg(test)`): a scripted MI
//! transport and a scripted HID transport, shared by the adapter seam tests
//! (tickets 03/04/05 reuse them).
//!
//! Scripting model: each key (namespace, class, method) maps to a queue of
//! outcomes. The LAST entry of a queue repeats forever; an empty queue is a
//! test bug and panics. Every received invocation is recorded so tests can
//! assert the exact wire protocol (input element names/values/types, in
//! order).
//!
//! Fakes are `Clone`, and clones share their interior state (`Rc`), so a
//! test can hand a clone to an adapter and keep the original to assert on
//! recorded calls afterwards.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::hid::{HidError, HidTransport};
use crate::mi::{MiError, MiResult};
use crate::policy::Profile;
use crate::power::{PlanApi, PowerError};
use crate::transport::{MiInput, MiOutput, MiTransport};

/// One recorded invocation of the scripted MI transport (namespace, class,
/// method + the typed input bag, verbatim).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordedCall {
    pub namespace: String,
    pub class: String,
    pub method: String,
    pub input: MiInput,
}

/// `Ok(None)` — a call that succeeded with no out-params instance.
pub fn no_output() -> Result<Option<MiOutput>, MiError> {
    Ok(None)
}

/// `Ok(Some(output))` — a call that returned an out-params instance.
pub fn some_output(output: MiOutput) -> Result<Option<MiOutput>, MiError> {
    Ok(Some(output))
}

/// `Err(MiError)` with the given result code (op = "FakeTransport").
pub fn transport_error(result: MiResult) -> Result<Option<MiOutput>, MiError> {
    Err(MiError { result, op: "FakeTransport", message: None })
}

/// One scripted outcome for an MI method key.
type ScriptedOutcome = Result<Option<MiOutput>, MiError>;
/// (namespace, class, method) → queue of scripted outcomes.
type ScriptMap = HashMap<(String, String, String), Vec<ScriptedOutcome>>;

/// State shared across `FakeTransport::connect()` calls: tests script what
/// the NEXT fresh transport serves (`script_next_connect`) and whether
/// `connect()` itself fails (`script_connect_outcomes`, one-shot queue —
/// drained, so a balanced test never leaks into the next one).
#[derive(Default)]
struct ConnectState {
    /// One-shot outcomes of the next `connect()` calls; empty → success.
    outcomes: Vec<Result<(), MiError>>,
    /// Script for the transport the next successful `connect()` produces.
    next_script: Option<ScriptMap>,
    /// Number of `connect()` invocations (for recorded-call-count asserts).
    calls: usize,
}

// Thread-local because `cargo test` runs each test on its own thread:
// reconnect tests script the next `connect()` without racing each other.
thread_local! {
    static CONNECT_STATE: RefCell<ConnectState> = const { RefCell::new(ConnectState { outcomes: Vec::new(), next_script: None, calls: 0 }) };
}

/// Scripted `MiTransport`: (namespace, class, method) → queue of outcomes;
/// the last entry of each queue repeats forever. Unscripted keys panic —
/// a missing script entry is a test bug. Records every invocation.
#[derive(Clone, Default)]
pub struct FakeTransport {
    inner: Rc<FakeTransportInner>,
}

#[derive(Default)]
struct FakeTransportInner {
    script: RefCell<ScriptMap>,
    calls: RefCell<Vec<RecordedCall>>,
}

impl FakeTransport {
    /// A new fake with an empty script.
    pub fn new() -> Self {
        Self::default()
    }

    /// Script `outcomes` for one (namespace, class, method) key; the last
    /// entry repeats forever. Replaces any previous script for the key.
    pub fn script(
        &self,
        namespace: &str,
        class: &str,
        method: &str,
        outcomes: impl IntoIterator<Item = Result<Option<MiOutput>, MiError>>,
    ) -> &Self {
        let entries: Vec<_> = outcomes.into_iter().collect();
        assert!(!entries.is_empty(), "FakeTransport: empty script for {namespace}\\{class}::{method}");
        self.inner.script.borrow_mut().insert(
            (namespace.to_string(), class.to_string(), method.to_string()),
            entries,
        );
        self
    }

    /// All recorded invocations, in call order (cloned out).
    pub fn calls(&self) -> Vec<RecordedCall> {
        self.inner.calls.borrow().clone()
    }

    /// Number of recorded invocations of `method` (any namespace/class).
    pub fn count(&self, method: &str) -> usize {
        self.inner.calls.borrow().iter().filter(|c| c.method == method).count()
    }

    /// Total number of recorded invocations.
    pub fn total(&self) -> usize {
        self.inner.calls.borrow().len()
    }

    /// Script one (namespace, class, method) key on the transport the NEXT
    /// successful `FakeTransport::connect()` produces (reconnect-path
    /// tests). The last scripted entry repeats forever; replaces any
    /// previous script for the key. Clears a pending connect failure.
    pub fn script_next_connect(
        &self,
        namespace: &str,
        class: &str,
        method: &str,
        outcomes: impl IntoIterator<Item = Result<Option<MiOutput>, MiError>>,
    ) -> &Self {
        let entries: Vec<_> = outcomes.into_iter().collect();
        assert!(!entries.is_empty(), "FakeTransport: empty next-connect script for {namespace}\\{class}::{method}");
        CONNECT_STATE.with(|state| {
            let mut state = state.borrow_mut();
            state.outcomes.clear();
            state
                .next_script
                .get_or_insert_with(Default::default)
                .insert((namespace.to_string(), class.to_string(), method.to_string()), entries);
        });
        self
    }

    /// Script the outcomes of the next `connect()` calls as a ONE-SHOT
    /// queue (drained, no repeat-last): `Ok(())` makes the next connect
    /// succeed, `Err(e)` makes it fail. An empty queue (default) means
    /// success. Every `script_next_connect` call clears any pending
    /// outcomes, so a test scripts exactly one behavior.
    pub fn script_connect_outcomes(
        &self,
        outcomes: impl IntoIterator<Item = Result<(), MiError>>,
    ) -> &Self {
        CONNECT_STATE.with(|state| {
            state.borrow_mut().outcomes = outcomes.into_iter().collect();
        });
        self
    }

    /// Number of `connect()` invocations (recorded on the thread-local
    /// connect registry, so reconnect tests can assert attempts by call
    /// count without inspecting logs).
    pub fn connect_count() -> usize {
        CONNECT_STATE.with(|state| state.borrow().calls)
    }
}

impl MiTransport for FakeTransport {
    fn connect() -> Result<Self, MiError> {
        CONNECT_STATE.with(|state| {
            let mut state = state.borrow_mut();
            state.calls += 1;
            if !state.outcomes.is_empty() {
                let outcome = state.outcomes.remove(0);
                match outcome {
                    Ok(()) => Ok(Self::connected(state.next_script.take())),
                    Err(err) => Err(err),
                }
            } else {
                Ok(Self::connected(state.next_script.take()))
            }
        })
    }

    fn invoke_first_instance(
        &self,
        namespace: &str,
        class: &str,
        method: &str,
        input: &MiInput,
    ) -> Result<Option<MiOutput>, MiError> {
        self.inner.calls.borrow_mut().push(RecordedCall {
            namespace: namespace.to_string(),
            class: class.to_string(),
            method: method.to_string(),
            input: input.clone(),
        });
        let key = (namespace.to_string(), class.to_string(), method.to_string());
        let mut script = self.inner.script.borrow_mut();
        let queue = script
            .get_mut(&key)
            .unwrap_or_else(|| panic!("no scripted outcome for {namespace}\\{class}::{method}"));
        pop_repeating(queue).expect("scripted queue is non-empty")
    }
}

impl FakeTransport {
    /// A fresh fake bound to the given script, for `connect()`.
    fn connected(script: Option<ScriptMap>) -> Self {
        Self {
            inner: Rc::new(FakeTransportInner {
                script: RefCell::new(script.unwrap_or_default()),
                calls: RefCell::new(Vec::new()),
            }),
        }
    }
}

/// Scripted `HidTransport`: queued `set_feature` outcomes (last repeats,
/// empty = `Io` error) and canned `get_feature` payloads (last repeats,
/// empty = `Io` error). Records every written report.
#[derive(Clone, Default)]
pub struct FakeHidTransport {
    inner: Rc<FakeHidTransportInner>,
}

#[derive(Default)]
struct FakeHidTransportInner {
    set_outcomes: RefCell<Vec<Result<(), HidError>>>,
    get_payloads: RefCell<Vec<[u8; 65]>>,
    sent_reports: RefCell<Vec<[u8; 65]>>,
}

impl FakeHidTransport {
    /// A new fake with an empty script.
    pub fn new() -> Self {
        Self::default()
    }

    /// Script `set_feature` outcomes; the last entry repeats forever.
    pub fn script_set(&self, outcomes: Vec<Result<(), HidError>>) -> &Self {
        assert!(!outcomes.is_empty(), "FakeHidTransport: empty set_feature script");
        *self.inner.set_outcomes.borrow_mut() = outcomes;
        self
    }

    /// Script canned `get_feature` payloads; the last entry repeats forever.
    /// An empty queue is an `Io` error.
    pub fn script_get(&self, payloads: Vec<[u8; 65]>) -> &Self {
        *self.inner.get_payloads.borrow_mut() = payloads;
        self
    }

    /// All 65-byte reports written via `set_feature`, in call order.
    pub fn sent_reports(&self) -> Vec<[u8; 65]> {
        self.inner.sent_reports.borrow().clone()
    }
}

impl HidTransport for FakeHidTransport {
    fn set_feature(&self, report: &[u8; 65]) -> Result<(), HidError> {
        self.inner.sent_reports.borrow_mut().push(*report);
        match pop_repeating(&mut self.inner.set_outcomes.borrow_mut()) {
            Some(outcome) => outcome,
            None => Err(HidError::Io { message: "FakeHidTransport: no scripted set_feature outcome".into() }),
        }
    }

    fn get_feature(&self, buffer: &mut [u8; 65]) -> Result<(), HidError> {
        match pop_repeating(&mut self.inner.get_payloads.borrow_mut()) {
            Some(payload) => {
                *buffer = payload;
                Ok(())
            }
            None => Err(HidError::Io { message: "FakeHidTransport: no scripted get_feature payload".into() }),
        }
    }
}

/// Pop the front of a script queue; when one entry remains it repeats
/// forever (never drained to empty). `None` for an empty queue.
fn pop_repeating<T: Clone>(queue: &mut Vec<T>) -> Option<T> {
    match queue.len() {
        0 => None,
        1 => Some(queue[0].clone()),
        _ => Some(queue.remove(0)),
    }
}

/// Scripted `PlanApi`: queued outcomes for `ensure_support`, `set_profile`
/// and `active_profile` (last entry repeats forever; an unscripted method
/// succeeds with a sensible default — the plan API is not the
/// panic-on-unscripted kind, failures are scripted explicitly). Records
/// every invocation so the apply/reconnect paths can be pinned by call
/// counts and arguments.
#[derive(Clone, Default)]
pub struct FakePlanApi {
    inner: Rc<FakePlanApiInner>,
}

#[derive(Default)]
struct FakePlanApiInner {
    support_outcomes: RefCell<Vec<Result<(), PowerError>>>,
    set_outcomes: RefCell<Vec<Result<(), PowerError>>>,
    profile_outcomes: RefCell<Vec<Result<Option<Profile>, PowerError>>>,
    support_calls: RefCell<usize>,
    set_calls: RefCell<Vec<Profile>>,
    profile_calls: RefCell<usize>,
}

impl FakePlanApi {
    /// A new fake with no scripted outcomes.
    pub fn new() -> Self {
        Self::default()
    }

    /// Script `ensure_support` outcomes; the last entry repeats forever.
    pub fn script_ensure(&self, outcomes: Vec<Result<(), PowerError>>) -> &Self {
        assert!(!outcomes.is_empty(), "FakePlanApi: empty ensure script");
        *self.inner.support_outcomes.borrow_mut() = outcomes;
        self
    }

    /// Script `set_profile` outcomes; the last entry repeats forever.
    pub fn script_set(&self, outcomes: Vec<Result<(), PowerError>>) -> &Self {
        assert!(!outcomes.is_empty(), "FakePlanApi: empty set script");
        *self.inner.set_outcomes.borrow_mut() = outcomes;
        self
    }

    /// Script `active_profile` outcomes; the last entry repeats forever.
    pub fn script_active_profile(&self, outcomes: Vec<Result<Option<Profile>, PowerError>>) -> &Self {
        assert!(!outcomes.is_empty(), "FakePlanApi: empty active-profile script");
        *self.inner.profile_outcomes.borrow_mut() = outcomes;
        self
    }

    /// Number of `ensure_support` invocations.
    pub fn ensure_calls(&self) -> usize {
        *self.inner.support_calls.borrow()
    }

    /// The profiles passed to `set_profile`, as their plan names, in call
    /// order.
    pub fn set_calls(&self) -> Vec<String> {
        self.inner.set_calls.borrow().iter().map(|p| p.plan_name().to_string()).collect()
    }

    /// Number of `active_profile` invocations.
    pub fn profile_calls(&self) -> usize {
        *self.inner.profile_calls.borrow()
    }
}

impl PlanApi for FakePlanApi {
    fn ensure_support(&self) -> Result<(), PowerError> {
        *self.inner.support_calls.borrow_mut() += 1;
        match pop_repeating(&mut self.inner.support_outcomes.borrow_mut()) {
            Some(outcome) => outcome,
            None => Ok(()),
        }
    }

    fn set_profile(&self, profile: Profile) -> Result<(), PowerError> {
        self.inner.set_calls.borrow_mut().push(profile);
        match pop_repeating(&mut self.inner.set_outcomes.borrow_mut()) {
            Some(outcome) => outcome,
            None => Ok(()),
        }
    }

    fn active_profile(&self) -> Result<Option<Profile>, PowerError> {
        *self.inner.profile_calls.borrow_mut() += 1;
        match pop_repeating(&mut self.inner.profile_outcomes.borrow_mut()) {
            Some(outcome) => outcome,
            None => Ok(Some(Profile::Balanced)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{FakeHidTransport, FakePlanApi};
    use crate::hid::HidTransport;
    use crate::mi::{MiError, MI_RESULT_FAILED, MI_RESULT_NOT_FOUND};
    use crate::policy::Profile;
    use crate::power::PlanApi;
    use crate::testing::{FakeTransport, no_output, some_output, transport_error};
    use crate::transport::{MiInput, MiOutput, MiTransport};

    const NS: &str = "ROOT\\WMI";
    const CLASS: &str = "AcerGamingFunction";

    #[test]
    fn scripted_outcome_is_served_and_invocation_recorded() {
        let fake = FakeTransport::new();
        let input = MiInput::new(CLASS).u64("gmInput", 0x60B);
        fake.script(NS, CLASS, "SetGamingMiscSetting", [no_output()]);
        let outcome = fake.invoke_first_instance(NS, CLASS, "SetGamingMiscSetting", &input);
        assert_eq!(outcome, Ok(None));
        let calls = fake.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].namespace, NS);
        assert_eq!(calls[0].class, CLASS);
        assert_eq!(calls[0].method, "SetGamingMiscSetting");
        assert_eq!(calls[0].input, input);
        assert_eq!(fake.count("SetGamingMiscSetting"), 1);
        assert_eq!(fake.total(), 1);
    }

    #[test]
    fn last_scripted_outcome_repeats_forever() {
        let fake = FakeTransport::new();
        let input = MiInput::new(CLASS).u32("gmInput", 0x0B);
        fake.script(NS, CLASS, "GetGamingMiscSetting", [
            transport_error(MI_RESULT_FAILED),
            no_output(),
        ]);
        assert!(fake.invoke_first_instance(NS, CLASS, "GetGamingMiscSetting", &input).is_err());
        assert_eq!(fake.invoke_first_instance(NS, CLASS, "GetGamingMiscSetting", &input), Ok(None));
        assert_eq!(fake.invoke_first_instance(NS, CLASS, "GetGamingMiscSetting", &input), Ok(None));
        assert_eq!(fake.count("GetGamingMiscSetting"), 3);
    }

    #[test]
    #[should_panic(expected = "no scripted outcome for ROOT\\WMI\\AcerGamingFunction::SetGamingMiscSetting")]
    fn unscripted_key_panics() {
        let fake = FakeTransport::new();
        let input = MiInput::new(CLASS);
        fake.invoke_first_instance(NS, CLASS, "SetGamingMiscSetting", &input).unwrap();
    }

    #[test]
    fn transport_error_is_echoed_with_its_result_code() {
        let fake = FakeTransport::new();
        fake.script(NS, CLASS, "GetGamingMiscSetting", [transport_error(MI_RESULT_NOT_FOUND)]);
        let input = MiInput::new(CLASS);
        let err = fake.invoke_first_instance(NS, CLASS, "GetGamingMiscSetting", &input).unwrap_err();
        assert_eq!(err.result, MI_RESULT_NOT_FOUND);
    }

    #[test]
    fn output_elements_are_echoed_verbatim() {
        let fake = FakeTransport::new();
        let output = MiOutput::new().with_u64("gmOutput", 0x600);
        fake.script(NS, CLASS, "GetGamingMiscSetting", [Ok(Some(output.clone()))]);
        let input = MiInput::new(CLASS);
        let got = fake.invoke_first_instance(NS, CLASS, "GetGamingMiscSetting", &input).unwrap();
        assert_eq!(got, Some(output));
    }

    #[test]
    fn hid_set_feature_scripts_and_records_reports() {
        let fake = FakeHidTransport::new();
        let err = crate::hid::HidError::Io { message: "boom".into() };
        fake.script_set(vec![Err(err.clone()), Err(err), Ok(())]);
        let mut report = [0u8; 65];
        report[0] = 0xA0;
        assert!(fake.set_feature(&report).is_err());
        assert!(fake.set_feature(&report).is_err());
        assert_eq!(fake.set_feature(&report), Ok(()));
        assert_eq!(fake.sent_reports().len(), 3);
        assert_eq!(fake.sent_reports()[2][0], 0xA0);
    }

    #[test]
    fn hid_get_feature_serves_canned_payloads() {
        let fake = FakeHidTransport::new();
        let mut payload = [0u8; 65];
        payload[8] = 2;
        fake.script_get(vec![payload]);
        let mut buffer = [0u8; 65];
        fake.get_feature(&mut buffer).unwrap();
        assert_eq!(buffer[8], 2);
        // One scripted payload repeats forever (repeat-last semantics).
        let mut again = [0u8; 65];
        fake.get_feature(&mut again).unwrap();
        assert_eq!(again[8], 2);
    }

    #[test]
    fn hid_get_feature_without_script_is_an_io_error() {
        let fake = FakeHidTransport::new();
        let mut buffer = [0u8; 65];
        assert!(fake.get_feature(&mut buffer).is_err());
    }

    #[test]
    fn hid_set_feature_without_script_is_an_io_error() {
        let fake = FakeHidTransport::new();
        let report = [0u8; 65];
        assert!(fake.set_feature(&report).is_err());
    }

    #[test]
    fn fake_connect_succeeds_with_the_seeded_script() {
        let fake = FakeTransport::new();
        fake.script_next_connect(NS, CLASS, "GetGamingMiscSetting", [
            some_output(MiOutput::new().with_u64("gmOutput", 0x600)),
        ]);
        let connected = FakeTransport::connect().unwrap();
        let input = MiInput::new(CLASS);
        assert_eq!(
            connected.invoke_first_instance(NS, CLASS, "GetGamingMiscSetting", &input).unwrap(),
            Some(MiOutput::new().with_u64("gmOutput", 0x600))
        );
        assert!(FakeTransport::connect_count() >= 1);
    }

    #[test]
    fn fake_connect_failure_is_one_shot_then_succeeds() {
        let fake = FakeTransport::new();
        fake.script_connect_outcomes([Err(MiError {
            result: MI_RESULT_FAILED,
            op: "FakeTransport",
            message: None,
        })]);
        assert!(FakeTransport::connect().is_err());
        assert!(FakeTransport::connect().is_ok());
    }

    #[test]
    fn fake_plan_api_records_and_scripts() {
        use crate::power::PowerError;
        let plan = FakePlanApi::new();
        plan.script_set(vec![Err(PowerError::NotFound("Nitro-X".into()))]);
        assert_eq!(plan.ensure_support(), Ok(()));
        assert_eq!(plan.set_profile(Profile::Quiet), Err(PowerError::NotFound("Nitro-X".into())));
        assert_eq!(plan.set_profile(Profile::Performance), Err(PowerError::NotFound("Nitro-X".into())));
        assert_eq!(plan.active_profile(), Ok(Some(Profile::Balanced)));
        assert_eq!(plan.ensure_calls(), 1);
        assert_eq!(plan.set_calls(), vec!["Nitro-Quiet".to_string(), "Nitro-Performance".to_string()]);
        assert_eq!(plan.profile_calls(), 1);
        plan.script_active_profile(vec![Ok(Some(Profile::Performance))]);
        assert_eq!(plan.active_profile(), Ok(Some(Profile::Performance)));
    }
}
