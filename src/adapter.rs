//! Shared circuit breaker, error type, and MI error mapping for the firmware
//! adapters (`wmi`, `charge`) — the one place where breaker policy, the
//! adapter error shape, and the MI→adapter error mapping change (ticket 04,
//! `.scratch/refactorings/issues/04-one-shared-circuit-breaker.md`).
//!
//! Both adapters stay thin over this module: every public adapter operation
//! runs through `CircuitBreaker::guarded`, which short-circuits to
//! `NotAvailable` once dead and counts failures to trip at
//! `MAX_ADAPTER_FAILURES`, mirroring the pre-ticket behavior exactly.

use std::cell::Cell;

use crate::log;
use crate::mi::{MiError, MI_RESULT_ACCESS_DENIED, MI_RESULT_INVALID_CLASS, MI_RESULT_NOT_FOUND};

/// Consecutive adapter failures after which the circuit breaker trips: a
/// flapping/starving provider must stop all further calls rather than keep
/// hammering a broken transport (the recovery loop reconnects the adapter).
pub const MAX_ADAPTER_FAILURES: u32 = 5;

/// WMI namespace shared by the firmware adapters (`charge`, `wmi`) — the one
/// place the protocol string lives; the probes import it from here instead of
/// carrying their own copies.
pub const WMI_NAMESPACE: &str = "ROOT\\WMI";

/// Errors from the firmware adapters (WMI gaming interface and smart
/// charge): the single error shape both adapters surface.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AdapterError {
    /// An MI/WMI call failed (hr carries the `MI_RESULT` code).
    Com { hr: i32, op: &'static str },
    /// The WMI interface was not found or is not accessible
    /// (interface unavailable).
    NotAvailable,
    /// An unexpected response shape.
    Unexpected(String),
}

/// Maps an `MiError` to `AdapterError`: interface-unavailable codes become
/// `NotAvailable` (the caller degrades), everything else is `Com`.
pub fn map_mi(err: MiError) -> AdapterError {
    match err.result {
        MI_RESULT_INVALID_CLASS | MI_RESULT_NOT_FOUND | MI_RESULT_ACCESS_DENIED => AdapterError::NotAvailable,
        _ => AdapterError::Com { hr: err.result, op: err.op },
    }
}

/// Failure-streak circuit breaker shared by the firmware adapters: counts
/// consecutive `guarded` failures, trips dead at `MAX_ADAPTER_FAILURES`
/// (logging the adapter's `trip_message`), and then refuses every call with
/// `NotAvailable` before it runs. A success resets the failure streak.
pub struct CircuitBreaker {
    /// Consecutive failed calls; trips the breaker at `MAX_ADAPTER_FAILURES`.
    failures: Cell<u32>,
    /// When set, every guarded call short-circuits to `NotAvailable`.
    dead: Cell<bool>,
    /// WARN line logged exactly once when the breaker trips (the owning
    /// adapter's degraded-mode message).
    trip_message: &'static str,
}

// The breaker is only ever touched from the adapter's owning thread, and the
// markers relax thread-safety claims for the single-threaded core that holds
// the adapter (COM no longer involved). Unsafe: the owning adapter
// serializes all access on its owning thread, and every `MiTransport`
// implementor in this crate is Send+Sync.
unsafe impl Send for CircuitBreaker {}
unsafe impl Sync for CircuitBreaker {}

impl CircuitBreaker {
    /// A fresh breaker: available, zero failures, and `trip_message` logged
    /// on the WARN level when it trips.
    pub fn new(trip_message: &'static str) -> Self {
        Self { failures: Cell::new(0), dead: Cell::new(false), trip_message }
    }

    /// Adapter still usable (breaker not tripped by a failure streak)?
    pub fn is_available(&self) -> bool {
        !self.dead.get()
    }

    /// Runs `call` guarded by the failure-streak circuit breaker: a dead
    /// breaker refuses the call with `NotAvailable` without running it; an
    /// error is counted and the breaker trips dead at
    /// `MAX_ADAPTER_FAILURES` consecutive errors; a success resets the
    /// failure streak.
    pub fn guarded<T>(
        &self,
        call: impl FnOnce() -> Result<T, AdapterError>,
    ) -> Result<T, AdapterError> {
        if self.dead.get() {
            return Err(AdapterError::NotAvailable);
        }
        let result = call();
        match &result {
            Ok(_) => self.failures.set(0),
            Err(_) => {
                let count = self.failures.get() + 1;
                self.failures.set(count);
                if count >= MAX_ADAPTER_FAILURES {
                    self.dead.set(true);
                    log::warn(self.trip_message);
                }
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;
    use crate::mi::{MiError, MI_RESULT_ACCESS_DENIED, MI_RESULT_INVALID_CLASS, MI_RESULT_INVALID_NAMESPACE, MI_RESULT_NOT_FOUND, MI_RESULT_TYPE_MISMATCH};

    fn com_error() -> AdapterError {
        AdapterError::Com { hr: 1, op: "t" }
    }

    #[test]
    fn breaker_trips_after_max_adapter_failures_consecutive_errors() {
        let breaker = CircuitBreaker::new("test: tripped");
        let mut last: Result<(), AdapterError> = Ok(());
        for _ in 0..MAX_ADAPTER_FAILURES {
            last = breaker.guarded(|| Err(com_error()));
        }
        // The threshold call itself ran and reported the error; the breaker
        // is dead afterwards and no further call runs.
        assert_eq!(last, Err(com_error()));
        assert!(!breaker.is_available());
    }

    #[test]
    fn breaker_resets_on_success_before_the_threshold() {
        let breaker = CircuitBreaker::new("test: tripped");
        for _ in 0..MAX_ADAPTER_FAILURES - 1 {
            let result: Result<(), AdapterError> = breaker.guarded(|| Err(com_error()));
            assert!(result.is_err());
        }
        assert!(breaker.is_available());
        assert!(breaker.guarded(|| Ok(())).is_ok());
        for _ in 0..MAX_ADAPTER_FAILURES - 1 {
            let result: Result<(), AdapterError> = breaker.guarded(|| Err(com_error()));
            assert!(result.is_err());
        }
        // 4 + 1 + 4 errors never reach the threshold: the success reset the streak.
        assert!(breaker.is_available());
    }

    #[test]
    fn dead_breaker_short_circuits_without_running_the_call() {
        let breaker = CircuitBreaker::new("test: tripped");
        for _ in 0..MAX_ADAPTER_FAILURES {
            let _: Result<(), AdapterError> = breaker.guarded(|| Err(com_error()));
        }
        let runs = Cell::new(0u32);
        let result: Result<(), AdapterError> = breaker.guarded(|| {
            runs.set(runs.get() + 1);
            Err(com_error())
        });
        assert_eq!(result, Err(AdapterError::NotAvailable));
        assert_eq!(runs.get(), 0);
        // A success through the dead breaker neither runs nor revives it.
        assert_eq!(breaker.guarded(|| Ok(())), Err(AdapterError::NotAvailable));
        assert!(!breaker.is_available());
    }

    #[test]
    fn is_available_reflects_the_breaker_state() {
        let breaker = CircuitBreaker::new("test: tripped");
        assert!(breaker.is_available());
        for _ in 0..MAX_ADAPTER_FAILURES {
            let _: Result<(), AdapterError> = breaker.guarded(|| Err(com_error()));
        }
        assert!(!breaker.is_available());
    }

    #[test]
    fn interface_unavailable_codes_map_to_not_available() {
        let err = MiError { result: MI_RESULT_INVALID_CLASS, op: "t", message: None };
        assert_eq!(map_mi(err), AdapterError::NotAvailable);
        let err = MiError { result: MI_RESULT_NOT_FOUND, op: "t", message: None };
        assert_eq!(map_mi(err), AdapterError::NotAvailable);
        let err = MiError { result: MI_RESULT_ACCESS_DENIED, op: "t", message: None };
        assert_eq!(map_mi(err), AdapterError::NotAvailable);
    }

    #[test]
    fn other_mi_codes_map_to_com() {
        let err = MiError { result: MI_RESULT_TYPE_MISMATCH, op: "MI_Instance_SetElement", message: None };
        assert_eq!(map_mi(err), AdapterError::Com { hr: 13, op: "MI_Instance_SetElement" });
        let err = MiError { result: MI_RESULT_INVALID_NAMESPACE, op: "t", message: None };
        assert_eq!(map_mi(err), AdapterError::Com { hr: 3, op: "t" });
    }
}
