use super::{MI_RESULT_NOT_FOUND, MiError};
use crate::transport::{MiInput, MiOutput, MiTransport};

/// Linux stub transport (linux-port ticket 02): mainline Linux has no generic
/// userspace WMI API, so the Acer firmware seam reports "unavailable" until
/// the ticket-03 kernel-module chardev transport lands. Keeps the same name
/// so `AppCore`'s default generic argument and the binary entry point stay
/// platform-agnostic; the real Linux transport replaces this body, nothing
/// else.
pub struct MiConnection;

/// The Linux stub's failure: `NOT_FOUND` (nothing to find yet) with the
/// ticket-02/03 pointer — one shared literal so `connect()` and
/// `invoke_first_instance` cannot drift apart.
fn linux_unavailable(op: &'static str) -> MiError {
    MiError {
        result: MI_RESULT_NOT_FOUND,
        op,
        message: Some("no WMI transport on Linux yet (ticket 02; chardev in ticket 03)".into()),
    }
}

impl MiTransport for MiConnection {
    fn connect() -> Result<Self, MiError> {
        Err(linux_unavailable("linux-mi"))
    }

    fn invoke_first_instance(
        &self,
        _namespace: &str,
        _class: &str,
        _method: &str,
        _input: &MiInput,
    ) -> Result<Option<MiOutput>, MiError> {
        Err(linux_unavailable("linux-mi"))
    }
}
