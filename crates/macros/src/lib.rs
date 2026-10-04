mod preload_themes;

use crate::preload_themes::preload_themes_impl;
use proc_macro::TokenStream;
use quote::quote;
use syn::{Expr, parse_file, parse_str};

/// Preload visual themes into memory at compile time.
///
/// Syntax:
///
/// ```Rust
///     preload_themes! {}
/// ```
///
#[proc_macro]
pub fn preload_themes(input: TokenStream) -> TokenStream {
    maybe_expand_proc_macro(false, "preload_themes", &input, preload_themes_impl)
}

fn maybe_expand_proc_macro<F>(
    expand: bool,
    name: &str,
    input: &TokenStream,
    proc_macro: F,
) -> TokenStream
where
    F: Fn(TokenStream) -> TokenStream,
{
    // Call the provided macro function
    let output = proc_macro(input.clone());

    if expand {
        expand_output(name, &output);
    }

    output
}

fn expand_output(name: &str, output: &TokenStream) {
    // Pretty-print the expanded tokens
    use inline_colorization::{color_cyan, color_reset, style_bold, style_reset, style_underline};
    let output: proc_macro2::TokenStream = output.clone().into();
    let token_str = output.to_string();
    let dash_line = "─".repeat(70);

    // First try to parse as a file
    match parse_file(&token_str) {
        Ok(syn_file) => {
            let pretty_output = prettyplease::unparse(&syn_file);
            eprintln!("{style_reset}{dash_line}{style_reset}");
            eprintln!(
                "{style_bold}{style_underline}Expanded macro{style_reset} {style_bold}{color_cyan}{name}{color_reset}:{style_reset}\n"
            );
            eprint!("{pretty_output}");
            eprintln!("{style_reset}{dash_line}{style_reset}");
        }
        // If parsing as a file fails, try parsing as an expression
        Err(_) => match parse_str::<Expr>(&token_str) {
            Ok(expr) => {
                // For expressions, we don't have a pretty printer, so just output the token string
                eprintln!("{style_reset}{dash_line}{style_reset}");
                eprintln!(
                    "{style_bold}{style_underline}Expanded macro{style_reset} {style_bold}{color_cyan}{name}{color_reset} (as expression):{style_reset}\n"
                );
                eprintln!("{}", quote!(#expr));
                eprintln!("{style_reset}{dash_line}{style_reset}");
            }
            Err(_e) => {
                // eprintln!("Failed to parse tokens as file or expression: {e:?}");
                eprintln!("{style_reset}{dash_line}{style_reset}");
                eprintln!(
                    "{style_bold}{style_underline}Expanded macro{style_reset} {style_bold}{color_cyan}{name}{color_reset} (as token string):{style_reset}\n"
                );
                eprintln!("{token_str}");
                eprintln!("{style_reset}{dash_line}{style_reset}");
            }
        },
    }
}
