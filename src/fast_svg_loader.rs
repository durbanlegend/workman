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
