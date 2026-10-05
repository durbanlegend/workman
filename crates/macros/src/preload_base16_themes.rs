#![allow(clippy::module_name_repetitions)]
use base16::Base16;
use proc_macro::TokenStream;
use quote::quote;
use std::env;

pub fn preload_base16_themes_impl(_input: TokenStream) -> TokenStream {
    // eprintln!("\ncurrent_dir={:?}", env::current_dir());
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");
    // eprintln!("The project manifest directory is: {manifest_dir}");
    let themes_dir = manifest_dir + "/assets/themes";
    // let themes_dir = "/Users/donf/projects/schemes-spec-0.11/base16/";

    let mut theme_mappings = Vec::new();

    for entry in std::fs::read_dir(themes_dir).unwrap() {
        let path = entry.unwrap().path();
        // Skip hidden files like .DS_Store and read only `.yaml` files
        if path.file_name().and_then(|n| n.to_str()).is_none_or(|n| {
            n.starts_with('.')
                || !std::path::Path::new(n)
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("yaml"))
        }) {
            continue;
        }

        let theme = Base16::from_file(&path).unwrap_or_else(|e| {
            eprintln!("failed to load {}: {e}", path.display());
            std::process::exit(1);
        });

        let key = &path.file_stem().unwrap().to_string_lossy().into_owned();
        let name = theme.name;
        let is_dark: bool = theme.is_dark;

        // 1. Map each Color32 in the array to a tokenized constructor expression
        let color_tokens: Vec<_> = theme
            .c
            .iter()
            .map(|color| {
                let [r, g, b, a] = color.to_srgba_unmultiplied();
                quote! {
                    egui::Color32::from_rgba_unmultiplied_const(#r, #g, #b, #a)
                }
            })
            .collect();

        // 2. Interpolate the expanded array tokens into the struct literal
        let theme_mapping = quote! {
            #key => Base16 {
                name: #name,
                is_dark: #is_dark,
                c: [ #(#color_tokens),* ],
            }
        };
        theme_mappings.push(theme_mapping);
    }

    // eprintln!("Done!");

    quote! {
        /// A static HashMap mapping theme names to preloaded themes
        static THEME_MAP: phf::Map<&'static str, Base16> = phf::phf_map! {
                #(#theme_mappings),*
            };
    }
    .into()
}
