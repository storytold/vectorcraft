//! Set the immutable nightly Windows identity (the taskbar group and jump-list id) before any native
//! window or COM service. Stable deliberately leaves its implicit shortcut identity unchanged, so the
//! MSI's shortcuts keep matching it.

use std::sync::OnceLock;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IdentityError {
    Configuration(String),
}

impl std::fmt::Display for IdentityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Configuration(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for IdentityError {}

pub fn initialize(nightly: bool, app_id: &str) -> Result<(), IdentityError> {
    // winsafe arrives with the wgpu renderer, which every desktop package builds with.
    configure(nightly && cfg!(windows) && cfg!(feature = "wgpu"), app_id, |id| {
        static RESULT: OnceLock<Result<(), String>> = OnceLock::new();
        RESULT
            .get_or_init(|| {
                #[cfg(all(windows, feature = "wgpu"))]
                {
                    winsafe::SetCurrentProcessExplicitAppUserModelID(id)
                        .map_err(|error| format!("cannot set the nightly Windows identity: {error}"))?;
                    let actual = winsafe::GetCurrentProcessExplicitAppUserModelID()
                        .map_err(|error| format!("cannot read back the nightly Windows identity: {error}"))?;
                    if actual != id {
                        return Err("the native Windows identity did not match the selected nightly channel".to_string());
                    }
                    Ok(())
                }
                #[cfg(not(all(windows, feature = "wgpu")))]
                {
                    let _ = id;
                    Ok(())
                }
            })
            .clone()
    })
    .map_err(IdentityError::Configuration)
}

fn configure(explicit_nightly: bool, app_id: &str, set: impl FnOnce(&str) -> Result<(), String>) -> Result<(), String> {
    if explicit_nightly { set(app_id) } else { Ok(()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_does_not_call_the_explicit_identity_api() {
        configure(false, "nightly-id", |_| panic!("stable identity must remain implicit")).unwrap();
    }

    #[test]
    fn nightly_applies_the_supplied_identity_once() {
        let calls = std::cell::Cell::new(0);
        configure(true, "nightly-id", |id| {
            assert_eq!(id, "nightly-id");
            calls.set(calls.get() + 1);
            Ok(())
        })
        .unwrap();
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn native_identity_failure_is_returned_instead_of_starting_a_window() {
        let started = std::cell::Cell::new(false);
        let result = configure(true, "nightly-id", |_| Err("native identity refused".to_string())).map(|()| started.set(true));
        assert_eq!(result.unwrap_err(), "native identity refused");
        assert!(!started.get());
    }
}
