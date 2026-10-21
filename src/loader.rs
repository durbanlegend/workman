//! Reading markdown files from disk for display.

use std::fs;
use std::path::Path;

/// Hard size limit: refuse files above this so the GUI doesn't freeze.
pub const MAX_BYTES: usize = 50_000_000;

/// The page shown in place of a file larger than [`MAX_BYTES`].
#[expect(clippy::cast_precision_loss)]
pub fn file_too_large_content(path: &Path, size_bytes: u64) -> String {
    let size_mb = size_bytes as f64 / 1e6;
    format!(
        "# \u{26a0} File Too Large\n\n\
         Cannot render `{}`.\n\n\
         **File size: {size_mb:.1} MB** \u{2014} exceeds the {:.0} MB limit.\n\n\
         Rendering files this large would make the UI unresponsive.\n\
         Consider splitting the file into smaller sections.",
        path.display(),
        MAX_BYTES as f64 / 1e6,
    )
}

/// Reads a markdown file for display.
///
/// Files over [`MAX_BYTES`] are never read into memory: the size is checked from the file's
/// metadata first, and the read itself is capped in case the file grows in the meantime. Such
/// a file yields the "too large" page instead, so that the watcher can still notice a fix.
/// Invalid UTF-8 is replaced rather than rejected, so that e.g. a Latin-1 file is still shown.
pub fn read_markdown_file(path: &Path) -> std::io::Result<String> {
    use std::io::Read;

    let too_large = |size: u64| {
        eprintln!("workman: file too large ({size} bytes): {}", path.display());
        Ok(file_too_large_content(path, size))
    };

    let file = fs::File::open(path)?;
    let len = file.metadata()?.len();
    if len > MAX_BYTES as u64 {
        return too_large(len);
    }
    let mut bytes = Vec::with_capacity(usize::try_from(len).unwrap_or(0));
    file.take(MAX_BYTES as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > MAX_BYTES {
        return too_large(bytes.len() as u64);
    }
    Ok(String::from_utf8(bytes)
        .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("workman-loader-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn small_file_is_read_verbatim() {
        let dir = temp_dir("small");
        let path = dir.join("a.md");
        fs::write(&path, "# Hello\n\nw\u{f6}rld\n").unwrap();
        assert_eq!(
            read_markdown_file(&path).unwrap(),
            "# Hello\n\nw\u{f6}rld\n"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_file_is_empty_string() {
        let dir = temp_dir("empty");
        let path = dir.join("a.md");
        fs::write(&path, "").unwrap();
        assert_eq!(read_markdown_file(&path).unwrap(), "");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn oversized_file_is_refused_without_reading_it() {
        let dir = temp_dir("oversized");
        let path = dir.join("big.md");
        // A sparse file: the size check must reject it from metadata alone.
        fs::File::create(&path)
            .unwrap()
            .set_len(MAX_BYTES as u64 + 1)
            .unwrap();
        let content = read_markdown_file(&path).unwrap();
        assert!(content.starts_with("# \u{26a0} File Too Large"));
        assert!(content.contains("big.md"));
        assert!(content.contains("50.0 MB"), "{content}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn file_at_the_limit_is_read() {
        let dir = temp_dir("at-limit");
        let path = dir.join("ok.md");
        fs::write(&path, vec![b'a'; MAX_BYTES]).unwrap();
        let content = read_markdown_file(&path).unwrap();
        assert_eq!(content.len(), MAX_BYTES);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn invalid_utf8_is_replaced_not_rejected() {
        let dir = temp_dir("latin1");
        let path = dir.join("latin1.md");
        fs::write(&path, b"caf\xe9\n").unwrap();
        assert_eq!(read_markdown_file(&path).unwrap(), "caf\u{fffd}\n");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_file_is_an_error() {
        let dir = temp_dir("missing");
        assert!(read_markdown_file(&dir.join("nope.md")).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn directory_is_an_error() {
        let dir = temp_dir("isdir");
        assert!(read_markdown_file(&dir).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn too_large_page_formats_size_and_limit() {
        let page = file_too_large_content(Path::new("/x/y.md"), 123_456_789);
        assert!(page.contains("`/x/y.md`"));
        assert!(page.contains("123.5 MB"));
        assert!(page.contains("50 MB limit"));
    }
}
