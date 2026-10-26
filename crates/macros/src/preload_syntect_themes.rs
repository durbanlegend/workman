#![allow(clippy::module_name_repetitions)]
use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::quote;
use std::{env, path::Path};

pub fn preload_syntect_themes_impl(_input: TokenStream) -> TokenStream {
    expand().unwrap_or_else(|e| e.to_compile_error().into())
}

fn expand() -> Result<TokenStream, syn::Error> {
    let fail = |msg: String| syn::Error::new(Span::call_site(), msg);

    let manifest_dir =
        env::var("CARGO_MANIFEST_DIR").map_err(|_| fail("CARGO_MANIFEST_DIR is not set".into()))?;
    let themes_dir = Path::new(&manifest_dir).join("assets/sublime_themes");

    let mut paths: Vec<_> = std::fs::read_dir(&themes_dir)
        .map_err(|e| fail(format!("cannot read {}: {e}", themes_dir.display())))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        // Skip hidden files like .DS_Store and read only `.tmTheme` files
        .filter(|path| {
            path.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                !n.starts_with('.')
                    && Path::new(n)
                        .extension()
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("tmTheme"))
            })
        })
        .collect();
    // A stable order keeps the generated code (and so the build) reproducible.
    paths.sort();

    let mut theme_mappings = Vec::new();
    for path in paths {
        let theme_name = path
            .file_stem()
            .ok_or_else(|| fail(format!("{} has no file name", path.display())))?
            .to_string_lossy()
            .into_owned();
        let theme_content_str = std::fs::read_to_string(&path)
            .map_err(|e| fail(format!("failed to load {}: {e}", path.display())))?;
        theme_mappings.push(quote! {
            #theme_name => #theme_content_str
        });
    }

    Ok(quote! {
        /// A static HashMap mapping TextMate theme names to preloaded themes
        /// for `egui_commonmark` to highlight code blocks using `syntect`
        static TM_THEME_MAP: phf::Map<&'static str, &'static str> = phf::phf_map! {
                #(#theme_mappings),*
            };
    }
    .into())
}
