use crate::policy::Profile;

use super::{PlanApi, PowerApi, PowerError};

/// Linux stub (linux-port ticket 02): plan enforcement runs quiet — every
/// operation succeeds without touching the OS, so no plan failure appears on
/// any enforce occasion and `effective()` shows no plan line. The sysfs
/// governor/EPP/boost backend (plan table 1:1, no external processes) lands
/// in ticket 05.
impl PlanApi for PowerApi {
    fn ensure_support(&self) -> Result<(), PowerError> {
        Ok(())
    }

    fn set_profile(&self, _profile: Profile) -> Result<(), PowerError> {
        Ok(())
    }

    fn active_profile(&self) -> Result<Option<Profile>, PowerError> {
        Ok(None)
    }
}
