use eframe::egui;
use egui::{
    Color32,
    load::{BytesPoll, ImageLoadResult, ImageLoader, ImagePoll, LoadError, SizeHint},
};
use std::{
    collections::HashMap,
    env,
    path::Path,
    sync::{Arc, Mutex},
};

// ─── Fast SVG loader ──────────────────────────────────────────────────────────
//
// `egui_extras::install_image_loaders` (called lazily by `egui_commonmark` on
// first render) only registers its built-in `SvgLoader` if no loader with that
// ID already exists:
//
//   if !ctx.is_loader_installed(SvgLoader::ID) { ctx.add_image_loader(...) }
//
// `SvgLoader::default()` calls `fontdb::Database::load_system_fonts()`, which
// on macOS scans `/System/Library/AssetsV2` — thousands of downloadable font
// assets — and takes ~20 seconds.
//
// By pre-registering `FastSvgLoader` with the **same ID**, we prevent that
// constructor from ever running.  Our loader calls `load_fonts_dir` on a small
// set of known directories, reading font files directly without the macOS
// CoreText/AssetsV2 scan, which is fast (<1 s).

pub struct FastSvgLoader {
    pub state: Mutex<FastSvgState>,
}

pub struct FastSvgState {
    pub pass_index: u64,
    pub cache: HashMap<String, HashMap<SizeHint, FastSvgEntry>>,
    pub options: resvg::usvg::Options<'static>,
}

pub struct FastSvgEntry {
    pub last_used: u64,
    pub result: Result<Arc<egui::ColorImage>, String>,
}

impl FastSvgLoader {
    /// Must match `egui::generate_loader_id!(SvgLoader)` as evaluated inside
    /// the `egui_extras::loaders::svg_loader` module:
    /// `concat!(module_path!(), "::", "SvgLoader")`
    pub const ID: &str = "egui_extras::loaders::svg_loader::SvgLoader";

    pub fn new() -> Self {
        let mut options = resvg::usvg::Options::default();

        // Populate fontdb from known directories instead of calling
        // `load_system_fonts()`.  On macOS this deliberately skips
        // `/System/Library/AssetsV2`, which is what causes the ~20 s delay.
        let db = options.fontdb_mut();

        #[cfg(target_os = "macos")]
        {
            db.load_fonts_dir("/System/Library/Fonts/");
            db.load_fonts_dir("/Library/Fonts/");
            if let Ok(home) = env::var("HOME") {
                db.load_fonts_dir(Path::new(&home).join("Library/Fonts"));
            }
        }
        #[cfg(target_os = "linux")]
        {
            db.load_fonts_dir("/usr/share/fonts/");
            db.load_fonts_dir("/usr/local/share/fonts/");
            if let Ok(home) = env::var("HOME") {
                db.load_fonts_dir(Path::new(&home).join(".fonts"));
                db.load_fonts_dir(Path::new(&home).join(".local/share/fonts"));
            }
        }
        #[cfg(target_os = "windows")]
        {
            db.load_fonts_dir("C:/Windows/Fonts/");
            if let Ok(profile) = env::var("USERPROFILE") {
                db.load_fonts_dir(
                    Path::new(&profile).join("AppData/Local/Microsoft/Windows/Fonts"),
                );
            }
        }

        log::info!(
            "FastSvgLoader: initialised ({} font faces)",
            options.fontdb.faces().count()
        );

        Self {
            state: Mutex::new(FastSvgState {
                pass_index: 0,
                cache: HashMap::default(),
                options,
            }),
        }
    }
}

impl ImageLoader for FastSvgLoader {
    fn id(&self) -> &str {
        Self::ID
    }

    fn load(&self, ctx: &egui::Context, uri: &str, size_hint: SizeHint) -> ImageLoadResult {
        if !Path::new(uri)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("svg"))
        {
            return Err(LoadError::NotSupported);
        }

        let FastSvgState {
            pass_index,
            cache,
            options,
        } = &mut *(self.state.lock().unwrap());

        let bucket = cache.entry(uri.to_owned()).or_default();
        if let Some(entry) = bucket.get_mut(&size_hint) {
            entry.last_used = *pass_index;
            return match entry.result.clone() {
                Ok(image) => Ok(ImagePoll::Ready { image }),
                Err(err) => Err(LoadError::Loading(err)),
            };
        }

        match ctx.try_load_bytes(uri) {
            Ok(BytesPoll::Ready { bytes, .. }) => {
                let result =
                    egui_extras::image::load_svg_bytes_with_size(&bytes, size_hint, options)
                        .map(Arc::new);
                bucket.insert(
                    size_hint,
                    FastSvgEntry {
                        last_used: *pass_index,
                        result: result.clone(),
                    },
                );
                match result {
                    Ok(image) => Ok(ImagePoll::Ready { image }),
                    Err(err) => Err(LoadError::Loading(err)),
                }
            }
            Ok(BytesPoll::Pending { size }) => Ok(ImagePoll::Pending { size }),
            Err(err) => Err(err),
        }
    }

    fn forget(&self, uri: &str) {
        self.state.lock().unwrap().cache.retain(|key, _| key != uri);
    }

    fn forget_all(&self) {
        self.state.lock().unwrap().cache.clear();
    }

    fn byte_size(&self) -> usize {
        self.state
            .lock()
            .unwrap()
            .cache
            .values()
            .flat_map(|bucket| bucket.values())
            .map(|entry| match &entry.result {
                Ok(image) => image.pixels.len() * std::mem::size_of::<Color32>(),
                Err(err) => err.len(),
            })
            .sum()
    }

    fn end_pass(&self, pass_index: u64) {
        let mut state = self.state.lock().unwrap();
        state.pass_index = pass_index;
        state.cache.retain(|_key, bucket| {
            if 2 <= bucket.len() {
                bucket.retain(|_, entry| pass_index <= entry.last_used + 1);
            }
            !bucket.is_empty()
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SVG: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"><rect width="4" height="4" fill="red"/></svg>"#;

    fn setup(uri: &'static str, bytes: &'static [u8]) -> (egui::Context, FastSvgLoader) {
        let ctx = egui::Context::default();
        ctx.include_bytes(uri, bytes);
        (ctx, FastSvgLoader::new())
    }

    fn cached_sizes(loader: &FastSvgLoader, uri: &str) -> usize {
        loader
            .state
            .lock()
            .unwrap()
            .cache
            .get(uri)
            .map_or(0, HashMap::len)
    }

    /// If this drifts, egui_commonmark would construct the 20-second default loader again.
    #[test]
    fn id_matches_the_loader_it_replaces() {
        assert_eq!(
            FastSvgLoader::ID,
            egui_extras::loaders::svg_loader::SvgLoader::ID
        );
        assert_eq!(FastSvgLoader::new().id(), FastSvgLoader::ID);
    }

    #[test]
    fn non_svg_uris_are_not_supported() {
        let (ctx, loader) = setup("bytes://a.png", b"x");
        for uri in [
            "bytes://a.png",
            "file:///x/y.jpg",
            "https://x/y",
            "no-extension",
        ] {
            assert!(
                matches!(
                    loader.load(&ctx, uri, SizeHint::default()),
                    Err(LoadError::NotSupported)
                ),
                "{uri}"
            );
        }
    }

    #[test]
    fn svg_extension_is_case_insensitive() {
        let (ctx, loader) = setup("bytes://UPPER.SVG", SVG);
        assert!(matches!(
            loader.load(&ctx, "bytes://UPPER.SVG", SizeHint::default()),
            Ok(ImagePoll::Ready { .. })
        ));
    }

    #[test]
    fn svg_is_rasterised_and_cached_per_size() {
        let (ctx, loader) = setup("bytes://a.svg", SVG);
        let Ok(ImagePoll::Ready { image }) = loader.load(&ctx, "bytes://a.svg", SizeHint::Width(8))
        else {
            panic!("expected a ready image");
        };
        assert_eq!(image.size, [8, 8]);
        assert_eq!(cached_sizes(&loader, "bytes://a.svg"), 1);

        // Same size: served from the cache; different size: a second entry.
        assert!(
            loader
                .load(&ctx, "bytes://a.svg", SizeHint::Width(8))
                .is_ok()
        );
        assert_eq!(cached_sizes(&loader, "bytes://a.svg"), 1);
        assert!(
            loader
                .load(&ctx, "bytes://a.svg", SizeHint::Width(16))
                .is_ok()
        );
        assert_eq!(cached_sizes(&loader, "bytes://a.svg"), 2);
        assert!(loader.byte_size() > 0);
    }

    #[test]
    fn invalid_svg_is_a_cached_loading_error() {
        let (ctx, loader) = setup("bytes://bad.svg", b"this is not svg");
        for _ in 0..2 {
            assert!(matches!(
                loader.load(&ctx, "bytes://bad.svg", SizeHint::default()),
                Err(LoadError::Loading(_))
            ));
        }
        assert_eq!(cached_sizes(&loader, "bytes://bad.svg"), 1);
    }

    #[test]
    fn forget_and_forget_all_drop_cache_entries() {
        let (ctx, loader) = setup("bytes://a.svg", SVG);
        ctx.include_bytes("bytes://b.svg", SVG);
        loader
            .load(&ctx, "bytes://a.svg", SizeHint::default())
            .unwrap();
        loader
            .load(&ctx, "bytes://b.svg", SizeHint::default())
            .unwrap();

        loader.forget("bytes://a.svg");
        assert_eq!(cached_sizes(&loader, "bytes://a.svg"), 0);
        assert_eq!(cached_sizes(&loader, "bytes://b.svg"), 1);

        loader.forget_all();
        assert_eq!(cached_sizes(&loader, "bytes://b.svg"), 0);
        assert_eq!(loader.byte_size(), 0);
    }

    #[test]
    fn end_pass_evicts_stale_sizes_but_keeps_a_lone_entry() {
        let (ctx, loader) = setup("bytes://a.svg", SVG);
        loader.end_pass(0);
        loader
            .load(&ctx, "bytes://a.svg", SizeHint::Width(8))
            .unwrap();
        loader
            .load(&ctx, "bytes://a.svg", SizeHint::Width(16))
            .unwrap();
        assert_eq!(cached_sizes(&loader, "bytes://a.svg"), 2);

        // Both used in pass 0; at pass 5 neither is recent, but only buckets with several
        // sizes are pruned, and a bucket that empties is removed.
        loader.end_pass(5);
        assert_eq!(cached_sizes(&loader, "bytes://a.svg"), 0);

        // A single entry is never pruned, however old (it is the only size in use).
        loader
            .load(&ctx, "bytes://a.svg", SizeHint::Width(8))
            .unwrap();
        loader.end_pass(50);
        assert_eq!(cached_sizes(&loader, "bytes://a.svg"), 1);
    }
}
