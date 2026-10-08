use crate::get_home_dir_string;

use documented::{Documented, DocumentedFields};
use edit::edit_file;
use mockall::{automock, predicate::str};
use serde::{Deserialize, Deserializer, Serialize, de::Error as DeError};
#[cfg(target_os = "windows")]
use std::env;
use std::{
    env::{current_dir, var},
    error::Error,
    fmt::Debug,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};
// use strum::{Display, EnumString};
use toml::Value;
use toml_edit::DocumentMut;

const DEFAULT_CONFIG: &str = include_str!("../assets/default_config.toml");

/// Configuration categories
#[derive(Clone, Debug, Default, Deserialize, Serialize, Documented, DocumentedFields)]
#[serde(default)]
pub struct Config {
    /// Theming
    pub theming: Theming,
    /// Miscellaneous settings
    pub misc: Misc,
}

/// Logging settings
#[derive(Clone, Debug, Default, Deserialize, Serialize, Documented, DocumentedFields)]
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
    /// Load the user's config file, or if there is none, load the default.
    ///
    /// # Errors
    ///
    /// This function will bubble up any i/o errors encountered.
    pub fn load_or_create_default(ctx: &impl Context) -> Result<Self, Box<dyn Error>> {
        let config_path = ctx.get_config_path();

        #[cfg(debug_assertions)]
        eprintln!(
            "1. config_path={}, exists={}",
            config_path.display(),
            config_path.exists()
        );

        if !config_path.exists() {
            let path = config_path.parent().ok_or_else(|| {
                ConfigError::Generic(format!("No parent for {}", config_path.display()))
            })?;
            fs::create_dir_all(path)?;

            // Try to find default config in different locations
            let default_config = if let Ok(cargo_home) = std::env::var("CARGO_HOME") {
                // First try cargo installed assets location
                let user_config = PathBuf::from(cargo_home)
                    .join("assets")
                    .join("default_config.toml");

                #[cfg(debug_assertions)]
                eprintln!(
                    "2. dist_config={}, exists={}",
                    config_path.display(),
                    user_config.exists()
                );
                if user_config.exists() {
                    fs::read_to_string(user_config)?
                } else {
                    // Fallback to embedded config
                    DEFAULT_CONFIG.to_string()
                }
            } else {
                DEFAULT_CONFIG.to_string()
            };

            #[cfg(debug_assertions)]
            eprintln!("3. default_config={default_config}");
            fs::write(&config_path, default_config)?;
        }

        #[cfg(debug_assertions)]
        eprintln!(
            "4. config_path={}, exists={}",
            config_path.display(),
            config_path.exists()
        );
        // let config_str = fs::read_to_string(&config_path)?;
        // let maybe_config = toml::from_str(&config_str);
        let maybe_config = Self::load(&config_path);

        #[cfg(debug_assertions)]
        eprintln!("5. maybe_config={maybe_config:#?}");
        Ok(maybe_config?)
    }

    /// Load a configuration.
    ///
    /// # Errors
    ///
    /// This function will bubble up any errors encountered.
    pub fn load(path: &Path) -> ConfigResult<Self> {
        let content = std::fs::read_to_string(path)?;

        match toml::from_str::<Self>(&content) {
            Ok(config) => {
                config.validate()?;
                validate_config_format(&content)?;
                Ok(config)
            }
            Err(e) => {
                // If parsing failed, try to salvage what we can
                eprintln!("Error: Config parse error ({e}).");
                Err(ConfigError::TomlParse(e))
            }
        }
    }

    #[expect(clippy::unnecessary_wraps, clippy::unused_self)]
    const fn validate(&self) -> ConfigResult<()> {
        // Add validation as needed
        Ok(())
    }
}

/// Miscellaneous configuration parameters
#[derive(Clone, Debug, Default, Documented, DocumentedFields, Deserialize, Serialize)]
pub struct Misc {}

/// Custom deserialisation method for booleans, to accept current true/false or legacy "true"/"false".
#[expect(dead_code)]
fn boolean<'de, D: Deserializer<'de>>(deserializer: D) -> Result<bool, D::Error> {
    let deserialize = Deserialize::deserialize(deserializer);
    deserialize.map_or_else(Err, |val| match val {
        Value::Boolean(b) => Ok(b),
        Value::String(s) => Ok(&s == "true"),
        _ => Err(DeError::custom("Wrong type, expected boolean")),
    })
}

#[automock]
/// Trait for providing configuration context, allowing for different implementations
/// in production versus testing environments.
pub trait Context: Debug {
    /// Returns the path where the configuration file should be located.
    fn get_config_path(&self) -> PathBuf;
    /// Returns true if this is a real context (not a mock for testing).
    fn is_real(&self) -> bool;
}

/// A struct for use in normal execution, as opposed to use in testing.
#[derive(Debug, Default)]
pub struct RealContext {
    /// Base directory for configuration files
    pub base_dir: PathBuf,
}

impl RealContext {
    /// Creates a new [`RealContext`].
    ///
    /// # Panics
    ///
    /// Panics if it fails to resolve the $APPDATA path.
    #[cfg(target_os = "windows")]
    #[must_use]
    pub fn new() -> Self {
        let base_dir =
            PathBuf::from(env::var("APPDATA").expect("Error resolving path from $APPDATA"));
        Self { base_dir }
    }

    /// Creates a new [`RealContext`].
    ///
    /// # Panics
    ///
    /// Panics if it fails to resolve the home directory.
    #[cfg(not(target_os = "windows"))]
    #[must_use]
    pub fn new() -> Self {
        let base_dir = PathBuf::from(get_home_dir_string().expect("Could not find home directory"))
            .join(".config");
        Self { base_dir }
    }
}

impl Context for RealContext {
    fn get_config_path(&self) -> PathBuf {
        let app_name = env!("CARGO_PKG_NAME");
        self.base_dir.join(app_name).join("config.toml")
    }

    fn is_real(&self) -> bool {
        true
    }
}

/// Initializes and returns the configuration.
#[allow(clippy::module_name_repetitions)]
pub fn maybe_config() -> Option<Config> {
    lazy_static_var!(Option<Config>, {
        let context = RealContext::new();
        let load_or_default = Config::load_or_create_default(&context);
        load_or_default.map_or_else(|_| maybe_load_config(), Some)
    })
    .clone()
}

fn maybe_load_config() -> Option<Config> {
    // eprintln!("In maybe_load_config, should not see this message more than once");

    let context = get_context();

    match load(&context) {
        Ok(Some(config)) => Some(config),
        Ok(None) => {
            eprintln!("No config file found - this is allowed");
            None
        }
        Err(e) => {
            println!("Failed to load config: {e}");
            // sleep(Duration::from_secs(1));
            // println!("Failed to load config: {e}");
            std::process::exit(1);
        }
    }
}

/// Gets the real or mock context according to whether test mode is detected via the `TEST_ENV` sstem variable.
///
/// # Panics
///
/// Panics if there is any issue accessing the current directory, e.g. if it doesn't exist or we don't have sufficient permissions to access it.
#[must_use]
pub fn get_context() -> Arc<dyn Context> {
    let context: Arc<dyn Context> = if var("TEST_ENV").is_ok() {
        let current_dir = current_dir().expect("Could not get current dir");
        let config_path = current_dir.join("tests/assets").join("config.toml");
        let mut mock_context = MockContext::default();
        mock_context
            .expect_get_config_path()
            .return_const(config_path);
        mock_context.expect_is_real().return_const(false);
        Arc::new(mock_context)
    } else {
        Arc::new(RealContext::new())
    };
    context
}

/// Load the existing configuration file, if one exists at the specified location.
/// The absence of a configuration file is not an error.
///
/// # Errors
///
/// This function will return an error if it either finds a file and fails to read it,
/// or reads the file and fails to parse it.
pub fn load(context: &Arc<dyn Context>) -> ConfigResult<Option<Config>> {
    let config_path = context.get_config_path();

    eprintln!("config_path={}", config_path.display());

    if !config_path.exists() {
        println!(
            "Configuration file path {} not found. No config loaded. System defaults will be used.",
            config_path.display()
        );
        return Ok(Some(Config::default()));
    }

    let config = Config::load(&config_path)?;

    // Log validation success
    eprintln!("Config validation successful");
    Ok(Some(config))
}

/// Open the configuration file in an editor.
/// # Errors
/// Will return `Err` if there is an error editing the file.
/// # Panics
/// Will panic if it can't create the parent directory for the configuration.
#[allow(clippy::unnecessary_wraps)]
pub fn open(context: &dyn Context) -> ConfigResult<Option<String>> {
    let config_path = context.get_config_path();
    eprintln!("config_path={}", config_path.display());

    let exists = config_path.exists();
    if !exists {
        let dir_path = &config_path
            .parent()
            .ok_or_else(|| ConfigError::Generic("Can't create directory".to_string()))?;
        fs::create_dir_all(dir_path)?;

        println!(
            "Configuration file path {} not found. Creating it.",
            config_path.display()
        );

        fs::write(&config_path, DEFAULT_CONFIG)?;
    }

    eprintln!("Editing {}...", config_path.display());
    if context.is_real() {
        edit_file(&config_path)?;
    }
    Ok(Some(String::from("End of edit")))
}

/// Validate the content of the `config.toml` file.
///
/// # Errors
///
/// This function will bubble up any Toml parsing errors encountered.
pub fn validate_config_format(content: &str) -> ConfigResult<()> {
    // Try to parse as generic TOML first
    let doc = content
        .parse::<DocumentMut>()
        .map_err(|e| ConfigError::Generic(format!("Invalid TOML syntax: {e}")))?;

    // Check for required sections
    if !doc.contains_key("theming") {
        return Err(ConfigError::Generic(
            "Missing [theming] section in config".into(),
        ));
    }

    // // Check for common mistakes
    // if let Some(table) = doc.get("theming").and_then(|v| v.as_table()) {
    //     for (key, value) in table {
    //         #[allow(clippy::single_match)]
    //         match key {
    //             "default_theme" => {
    //                 if let Some(v) = value.as_str()
    //                     && v.chars().next().unwrap_or('_').is_uppercase()
    //                 {
    //                     return Err(ConfigError::Generic(format!(
    //                         "inference_level should be lowercase: '{v}' should be '{}'",
    //                         v.to_lowercase()
    //                     )));
    //                 }
    //             }
    //             // Add checks for other fields
    //             _ => {}
    //         }
    //     }
    // }

    Ok(())
}

/// Main function for use by testing or the script runner.
#[expect(dead_code, unused_variables)]
fn main() {
    let maybe_config = load(&get_context());

    if let Ok(Some(config)) = maybe_config {
        // #[cfg(debug_assertions)]
        // eprintln!("Loaded config: {config:?}");
    } else {
        eprintln!("No configuration file found.");
    }
}
