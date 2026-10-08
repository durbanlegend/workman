#![allow(clippy::module_name_repetitions)]
use proc_macro::TokenStream;
use quote::quote;
use std::env;

pub fn preload_syntect_themes_impl(_input: TokenStream) -> TokenStream {
    // eprintln!("\ncurrent_dir={:?}", env::current_dir());
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");
    // eprintln!("The project manifest directory is: {manifest_dir}");
    let themes_dir = manifest_dir + "/assets/sublime_themes";
    // let themes_dir = "/Users/donf/projects/TextMate-Themes";

    let mut theme_mappings = Vec::new();

    for entry in std::fs::read_dir(themes_dir).unwrap() {
        let path = entry.unwrap().path();
        // Skip hidden files like .DS_Store and read only `.tmTheme` files
        if path.file_name().and_then(|n| n.to_str()).is_none_or(|n| {
            n.starts_with('.')
                || !std::path::Path::new(n)
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("tmTheme"))
        }) {
            continue;
        }

        let theme = path.file_stem().unwrap().to_string_lossy().to_string();

        // let mut path = PathBuf::from_str(CARGO_MANIFEST_DIR).unwrap().push("/assets/sublime_themes/").push(theme_name);
        let theme_str = std::fs::read_to_string(&path).unwrap_or_else(|e| {
            eprintln!("failed to load {}: {e}", path.display());
            std::process::exit(1);
        });

        // let theme_bytes = theme_str.as_bytes();

        let theme_mapping = quote! {
            #theme => #theme_str
        };
        theme_mappings.push(theme_mapping);
    }

    quote! {
        /// A static HashMap mapping theme names to preloaded themes
        static SYNTECT_THEME_MAP: phf::Map<&'static str, &'static str> = phf::phf_map! {
                #(#theme_mappings),*
            };
    }
    .into()
}
