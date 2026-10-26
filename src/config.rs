//! The user's `config.toml`.

use crate::get_home_dir_string;

use serde::{Deserialize, Serialize};
use std::{
    fmt::Debug,
    fs,
    path::{Path, PathBuf},
    sync::OnceLock,
};
use toml_edit::DocumentMut;

const DEFAULT_CONFIG: &str = include_str!("../assets/default_config.toml");

/// Configuration categories
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Config {
    /// Theming
    pub theming: Theming,
    // /// Miscellaneous settings
    // pub misc: Misc,
}

/// Theming settings
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Theming {
    /// An optional default `base16` (`.yaml|.yml`) theme to use for the viewer and markdown document.
    /// This must be a built-in theme or reside directly under the directory specified by `base16_dir`.
    pub default_theme: Option<String>,
    /// An optional directory containing the specified  `default_theme` if not built in
    pub base16_dir: Option<String>,
    /// An optional default `TextMate` theme to use for dark-mode `syntect` code block highlighting
    pub default_tm_theme_dark: Option<String>,
    /// An optional default `TextMate` theme to use for light-mode `syntect` code block highlighting
    pub default_tm_theme_light: Option<String>,
    /// An optional directory containing the specified  `default_tm_theme_dark` and/or
    /// `default_tm_theme_light` if not built in
    pub tm_theme_dir: Option<String>,
}

impl Theming {
    #[must_use]
    pub const fn new(
        default_theme: Option<String>,
        base16_dir: Option<String>,
        default_tm_theme_dark: Option<String>,
        default_tm_theme_light: Option<String>,
        tm_theme_dir: Option<String>,
    ) -> Self {
        Self {
            default_theme,
            base16_dir,
            default_tm_theme_dark,
            default_tm_theme_light,
            tm_theme_dir,
        }
    }
}

/// Result type alias for config operations
pub type ConfigResult<T> = Result<T, ConfigError>;

/// Config-specific error types
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// IO error
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    /// TOML parsing error
    #[error("TOML parsing error: {0}")]
    TomlParse(#[from] toml::de::Error),
    /// TOML edit error
    #[error("TOML edit error: {0}")]
    TomlEdit(#[from] toml_edit::TomlError),
    /// Generic error
    #[error("{0}")]
    Generic(String),
}

impl Config {
    /// Load the user's config file, creating it from the built-in default first if there
    /// isn't one.
    ///
    /// # Errors
    ///
    /// Fails if the file can't be created, read or parsed.
    pub fn load_or_create_default(ctx: &impl Context) -> ConfigResult<Self> {
        let config_path = ctx.get_config_path();

        if !config_path.exists() {
            let dir = config_path.parent().ok_or_else(|| {
                ConfigError::Generic(format!("No parent for {}", config_path.display()))
            })?;
            fs::create_dir_all(dir)?;
            fs::write(&config_path, DEFAULT_CONFIG)?;
        }

        Self::load(&config_path)
    }

    /// Load a configuration.
    ///
    /// # Errors
    ///
    /// Fails if the file can't be read, isn't valid TOML, or lacks a `[theming]` section.
    pub fn load(path: &Path) -> ConfigResult<Self> {
        let content = fs::read_to_string(path)?;
        let config = toml::from_str::<Self>(&content)?;
        validate_config_format(&content)?;
        Ok(config)
    }
}

/// Provides the location of the configuration file, so that tests can use a scratch directory.
pub trait Context: Debug {
    /// Returns the path where the configuration file should be located.
    fn get_config_path(&self) -> PathBuf;
}

/// The real location of the configuration file: under the user's home directory (or
/// `%APPDATA%` on Windows).
#[derive(Debug)]
pub struct RealContext {
    /// Base directory for configuration files
    pub base_dir: PathBuf,
}

impl RealContext {
    /// Creates a new [`RealContext`], or `None` if the user's configuration directory can't
    /// be determined.
    #[cfg(target_os = "windows")]
    #[must_use]
    pub fn new() -> Option<Self> {
        let base_dir = PathBuf::from(std::env::var_os("APPDATA")?);
        Some(Self { base_dir })
    }

    /// Creates a new [`RealContext`], or `None` if the user's configuration directory can't
    /// be determined.
    #[cfg(not(target_os = "windows"))]
    #[must_use]
    pub fn new() -> Option<Self> {
        let base_dir = PathBuf::from(get_home_dir_string().ok()?).join(".config");
        Some(Self { base_dir })
    }
}

impl Context for RealContext {
    fn get_config_path(&self) -> PathBuf {
        self.base_dir.join(crate::APP_NAME).join("config.toml")
    }
}

/// Message describing why the user's config file could not be used, if that happened.
static CONFIG_ERROR: OnceLock<String> = OnceLock::new();

/// Initializes and returns the configuration.
///
/// Returns `None` if there is a config file that could not be read or parsed. In that case the
/// application carries on with its built-in defaults, and the reason is available from
/// [`config_error`] so that the UI can tell the user. (This used to terminate the process,
/// which, for a GUI relaunched with its stdio detached, meant the window silently never appeared.)
#[expect(clippy::module_name_repetitions)]
pub fn maybe_config() -> Option<Config> {
    lazy_static_var!(Option<Config>, {
        let Some(context) = RealContext::new() else {
            eprintln!("workman: cannot locate the configuration directory; using defaults");
            return Some(Config::default());
        };
        let (config, error) = resolve_config(&context);
        if let Some(error) = error {
            eprintln!("workman: {error}");
            let _ = CONFIG_ERROR.set(error);
        }
        config
    })
    .clone()
}

/// The reason the user's config file was ignored, if it was. Only meaningful after
/// [`maybe_config`] has been called.
#[must_use]
pub fn config_error() -> Option<&'static str> {
    CONFIG_ERROR.get().map(String::as_str)
}

/// Loads (creating if necessary) the configuration for `context`.
///
/// Returns the configuration to use plus, if the user's file was present but unusable,
/// a description of the problem. An unusable file yields `None` rather than defaults so
/// that nothing downstream mistakes the defaults for the user's settings and overwrites
/// the file. If the file doesn't exist and can't be created (e.g. a read-only home
/// directory), the defaults are used and there is no error.
fn resolve_config(context: &impl Context) -> (Option<Config>, Option<String>) {
    match Config::load_or_create_default(context) {
        Ok(config) => (Some(config), None),
        Err(e) if context.get_config_path().exists() => {
            let path = context.get_config_path();
            (
                None,
                Some(format!(
                    "cannot use config file {}: {e}. Using built-in defaults.",
                    path.display()
                )),
            )
        }
        Err(e) => {
            eprintln!("workman: no config file and could not create one ({e}); using defaults");
            (Some(Config::default()), None)
        }
    }
}

/// Validate the content of the `config.toml` file.
///
/// # Errors
///
/// Fails if the text isn't valid TOML or lacks a `[theming]` section.
pub fn validate_config_format(content: &str) -> ConfigResult<()> {
    let doc = content
        .parse::<DocumentMut>()
        .map_err(|e| ConfigError::Generic(format!("Invalid TOML syntax: {e}")))?;

    if !doc.contains_key("theming") {
        return Err(ConfigError::Generic(
            "Missing [theming] section in config".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct TestContext(PathBuf);

    impl Context for TestContext {
        fn get_config_path(&self) -> PathBuf {
            self.0.clone()
        }
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("workman-cfg-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn valid_config_is_used() {
        let dir = temp_dir("valid");
        let path = dir.join("config.toml");
        fs::write(&path, "[theming]\ndefault_theme = \"foo\"\n").unwrap();
        let (config, error) = resolve_config(&TestContext(path));
        assert_eq!(
            config.unwrap().theming.default_theme.as_deref(),
            Some("foo")
        );
        assert!(error.is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn broken_config_is_reported_not_fatal_and_left_untouched() {
        let dir = temp_dir("broken");
        let path = dir.join("config.toml");
        fs::write(&path, "[theming\nthis is not toml").unwrap();
        let (config, error) = resolve_config(&TestContext(path.clone()));
        assert!(config.is_none(), "no config must be assumed");
        let error = error.expect("the problem must be reported");
        assert!(error.contains("config.toml"), "{error}");
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "[theming\nthis is not toml",
            "the user's file must not be overwritten"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_config_is_created_from_the_default() {
        let dir = temp_dir("missing");
        let path = dir.join("sub").join("config.toml");
        let (config, error) = resolve_config(&TestContext(path.clone()));
        assert!(config.is_some());
        assert!(error.is_none());
        assert!(path.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn config_without_theming_section_is_rejected() {
        assert!(validate_config_format("foo = 1\n").is_err());
        assert!(validate_config_format("[theming]\n").is_ok());
    }

    #[test]
    fn shipped_default_config_loads_and_validates() {
        let dir = temp_dir("default");
        let path = dir.join("config.toml");
        fs::write(&path, DEFAULT_CONFIG).unwrap();
        let config = Config::load(&path).expect("the shipped default config must be valid");
        assert!(config.theming.default_theme.is_some());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn existing_config_is_never_overwritten() {
        let dir = temp_dir("keep");
        let path = dir.join("config.toml");
        let mine = "[theming]\ndefault_tm_theme_dark = \"Mine\"\n";
        fs::write(&path, mine).unwrap();
        let (config, _) = resolve_config(&TestContext(path.clone()));
        assert_eq!(
            config.unwrap().theming.default_tm_theme_dark.as_deref(),
            Some("Mine")
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), mine);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn unknown_keys_and_empty_sections_are_tolerated() {
        let dir = temp_dir("lenient");
        let path = dir.join("config.toml");
        fs::write(&path, "[theming]\n").unwrap();
        assert!(Config::load(&path).is_ok());
        fs::write(&path, "[theming]\nsome_future_key = 1\n").unwrap();
        assert!(Config::load(&path).is_ok());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn wrong_types_are_a_parse_error() {
        let dir = temp_dir("types");
        let path = dir.join("config.toml");
        fs::write(&path, "[theming]\ndefault_theme = 42\n").unwrap();
        assert!(matches!(
            Config::load(&path),
            Err(ConfigError::TomlParse(_))
        ));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_file_is_an_io_error() {
        assert!(matches!(
            Config::load(Path::new("/definitely/not/here.toml")),
            Err(ConfigError::Io(_))
        ));
    }

    #[test]
    fn real_context_path_is_under_base_dir_with_the_app_name() {
        let ctx = RealContext {
            base_dir: PathBuf::from("/base"),
        };
        assert_eq!(
            ctx.get_config_path(),
            Path::new("/base").join(crate::APP_NAME).join("config.toml")
        );
    }

    #[test]
    fn config_round_trips_through_toml() {
        let config = Config {
            theming: Theming::new(
                Some("a".into()),
                Some("/b".into()),
                Some("c".into()),
                Some("d".into()),
                Some("/e".into()),
            ),
        };
        let text = toml::to_string_pretty(&config).unwrap();
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(back.theming.default_theme.as_deref(), Some("a"));
        assert_eq!(back.theming.tm_theme_dir.as_deref(), Some("/e"));
        assert!(validate_config_format(&text).is_ok());
    }
}
