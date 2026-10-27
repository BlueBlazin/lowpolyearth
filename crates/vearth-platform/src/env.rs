//! Configuration variables: the process environment natively, and in the
//! browser the page's `VEARTH_*` URL query parameters, so the same
//! diagnostic switches work on both hosts.

#[cfg(not(target_arch = "wasm32"))]
pub use std::env::var;

#[cfg(target_arch = "wasm32")]
pub use web::{set_vars, var};

#[cfg(target_arch = "wasm32")]
mod web {
    use std::collections::HashMap;
    use std::env::VarError;
    use std::sync::OnceLock;

    static VARS: OnceLock<HashMap<String, String>> = OnceLock::new();

    /// Sets the variables once, before any are read.
    pub fn set_vars(vars: impl IntoIterator<Item = (String, String)>) {
        let _ = VARS.set(vars.into_iter().collect());
    }

    pub fn var(key: &str) -> Result<String, VarError> {
        VARS.get()
            .and_then(|vars| vars.get(key).cloned())
            .ok_or(VarError::NotPresent)
    }
}
