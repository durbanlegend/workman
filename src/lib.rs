use crate::config::ConfigError;
use std::error::Error;

/// Lazy-static variable generator.
///
/// Syntax:
/// ```ignore
/// use crate::lazy_static_var;
/// let my_var = lazy_static_var!(<T>, expr<T>); // for static ref
/// // or
/// let my_var = lazy_static_var!(<T>, deref, expr<T>); // for Deref value (not guaranteed)
/// ```///
/// NB: In order to avoid fighting the compiler, it is not recommended to make `my_var` uppercase.
// #[macro_export]
macro_rules! lazy_static_var {
    ($type:ty, deref, $init_fn:expr) => {{
        use std::sync::OnceLock;
        static GENERIC_LAZY: OnceLock<$type> = OnceLock::new();
        *GENERIC_LAZY.get_or_init(|| $init_fn)
    }};
    ($type:ty, $init_fn:expr) => {{
        use std::sync::OnceLock;
        static GENERIC_LAZY: OnceLock<$type> = OnceLock::new();
        GENERIC_LAZY.get_or_init(|| $init_fn)
    }};
}

/// Get the user's home directory as a `String`.
///
/// # Errors
///
/// This function will return an error if it can't resolve the user directories.
pub fn get_home_dir_string() -> Result<String, Box<dyn Error>> {
    let home_dir = dirs::home_dir()
        .ok_or_else(|| ConfigError::Generic("Could not resolve user home directory".into()))?;
    Ok(home_dir.display().to_string())
}

// Positioning it here ensures everything above is registered in the root before the compiler steps inside config.rs.
pub mod config;

pub mod html_prep;
pub mod html_render;
