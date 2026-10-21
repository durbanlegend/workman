//! Command-line and environment handling that can be decided without a GUI.

use std::fmt;
use std::path::{Path, PathBuf};

/// File extensions (lower case) that are treated as markdown.
pub const MARKDOWN_EXTENSIONS: &[&str] = &["md", "markdown"];

/// Whether `path` has a markdown file extension (case-insensitive).
#[must_use]
pub fn is_markdown_path(path: &Path) -> bool {
    path.extension().is_some_and(|ext| {
        MARKDOWN_EXTENSIONS
            .iter()
            .any(|m| ext.eq_ignore_ascii_case(m))
    })
}

/// What the positional command-line argument asks us to open.
#[derive(Debug, PartialEq, Eq)]
pub enum FileArg {
    /// No argument: start with the welcome page.
    None,
    /// A markdown file to open.
    File(PathBuf),
    /// A directory: start with the welcome page, with the directory as working directory.
    Directory(PathBuf),
}

/// A command-line argument that can't be used.
#[derive(Debug, PartialEq, Eq)]
pub enum FileArgError {
    NotFound(PathBuf),
    UnsupportedExtension(PathBuf),
}

impl fmt::Display for FileArgError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound(p) => write!(f, "Input file does not exist: {}", p.display()),
            Self::UnsupportedExtension(p) => write!(
                f,
                "Input file has unsupported extension: {}; must be one of: {}",
                p.display(),
                MARKDOWN_EXTENSIONS.join(", ")
            ),
        }
    }
}

impl std::error::Error for FileArgError {}

/// Classifies the first positional argument (if any).
///
/// # Errors
///
/// Returns an error if the path doesn't exist or is a file without a markdown extension.
pub fn resolve_file_arg(arg: Option<&str>) -> Result<FileArg, FileArgError> {
    let Some(arg) = arg else {
        return Ok(FileArg::None);
    };
    let path = Path::new(arg);
    if !path.exists() {
        Err(FileArgError::NotFound(path.to_path_buf()))
    } else if path.is_dir() {
        Ok(FileArg::Directory(path.to_path_buf()))
    } else if is_markdown_path(path) {
        Ok(FileArg::File(path.to_path_buf()))
    } else {
        Err(FileArgError::UnsupportedExtension(path.to_path_buf()))
    }
}

/// Reduces a raw locale string (`fr_FR.UTF-8`, `en-GB`, `nb_NO@euro`, ...) to the bare
/// language code that the translation tables are keyed by (`fr`, `en`, `nb`).
///
/// Returns `None` for values that mean "no locale" (empty, `C`, `POSIX`).
#[must_use]
pub fn normalise_locale(raw: &str) -> Option<String> {
    let base = raw.split(['.', '@']).next().unwrap_or("");
    let language = base
        .split(['_', '-'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    match language.as_str() {
        "" | "c" | "posix" => None,
        // Norwegian: the translations are keyed `nb`.
        "no" | "nn" => Some("nb".to_owned()),
        _ => Some(language),
    }
}

/// Chooses the UI locale: `$LOCALE`, then `$LANG`, then the operating system's setting,
/// then English. `system` is only called if the environment doesn't decide.
pub fn choose_locale(
    locale_env: Option<&str>,
    lang_env: Option<&str>,
    system: impl FnOnce() -> Option<String>,
) -> String {
    locale_env
        .and_then(normalise_locale)
        .or_else(|| lang_env.and_then(normalise_locale))
        .or_else(|| system().as_deref().and_then(normalise_locale))
        .unwrap_or_else(|| "en".to_owned())
}

/// Detect the preferred UI locale from the environment and operating system.
///
/// `sys-locale` reads native OS APIs (`CFPreferences` on macOS, `GetUserDefaultLocaleName`
/// on Windows, POSIX environment variables on Linux).
#[must_use]
pub fn detect_locale() -> String {
    choose_locale(
        std::env::var("LOCALE").ok().as_deref(),
        std::env::var("LANG").ok().as_deref(),
        sys_locale::get_locale,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("workman-cli-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn markdown_extensions_are_case_insensitive() {
        assert!(is_markdown_path(Path::new("a.md")));
        assert!(is_markdown_path(Path::new("a.MD")));
        assert!(is_markdown_path(Path::new("dir/a.Markdown")));
        assert!(!is_markdown_path(Path::new("a.txt")));
        assert!(!is_markdown_path(Path::new("md")));
        assert!(!is_markdown_path(Path::new("a.md.bak")));
    }

    #[test]
    fn no_argument_means_welcome_page() {
        assert_eq!(resolve_file_arg(None), Ok(FileArg::None));
    }

    #[test]
    fn resolve_file_arg_cases() {
        let dir = temp_dir("args");
        let md = dir.join("x.MD");
        let txt = dir.join("x.txt");
        fs::write(&md, "# hi").unwrap();
        fs::write(&txt, "hi").unwrap();

        assert_eq!(resolve_file_arg(md.to_str()), Ok(FileArg::File(md.clone())));
        assert_eq!(
            resolve_file_arg(dir.to_str()),
            Ok(FileArg::Directory(dir.clone()))
        );
        assert_eq!(
            resolve_file_arg(txt.to_str()),
            Err(FileArgError::UnsupportedExtension(txt.clone()))
        );
        let missing = dir.join("missing.md");
        assert_eq!(
            resolve_file_arg(missing.to_str()),
            Err(FileArgError::NotFound(missing.clone()))
        );
        assert!(
            FileArgError::NotFound(missing)
                .to_string()
                .contains("does not exist")
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn normalise_locale_cases() {
        assert_eq!(normalise_locale("fr_FR.UTF-8").as_deref(), Some("fr"));
        assert_eq!(normalise_locale("en-GB").as_deref(), Some("en"));
        assert_eq!(normalise_locale("DE").as_deref(), Some("de"));
        assert_eq!(normalise_locale("nb_NO@euro").as_deref(), Some("nb"));
        assert_eq!(normalise_locale("no").as_deref(), Some("nb"));
        assert_eq!(normalise_locale("no-NO").as_deref(), Some("nb"));
        assert_eq!(normalise_locale("nn_NO").as_deref(), Some("nb"));
        assert_eq!(normalise_locale("C"), None);
        assert_eq!(normalise_locale("C.UTF-8"), None);
        assert_eq!(normalise_locale("POSIX"), None);
        assert_eq!(normalise_locale(""), None);
    }

    #[test]
    fn choose_locale_precedence() {
        let sys = || Some("sv-SE".to_owned());
        assert_eq!(choose_locale(Some("fr"), Some("de_DE.UTF-8"), sys), "fr");
        assert_eq!(choose_locale(None, Some("de_DE.UTF-8"), sys), "de");
        // "C" says nothing, so fall through to the OS setting.
        assert_eq!(choose_locale(None, Some("C"), sys), "sv");
        assert_eq!(choose_locale(None, None, sys), "sv");
        assert_eq!(choose_locale(None, None, || None), "en");
        assert_eq!(choose_locale(Some(""), Some("POSIX"), || None), "en");
    }

    #[test]
    fn system_locale_is_not_consulted_when_env_decides() {
        let l = choose_locale(Some("fr"), None, || panic!("must not be called"));
        assert_eq!(l, "fr");
    }
}
