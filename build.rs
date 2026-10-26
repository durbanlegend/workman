//! The theme tables are baked into the binary by proc macros that read these directories.
//! Proc macros can't ask cargo to track their inputs on stable Rust, so do it here: a change
//! to (or addition of) a theme file rebuilds the crate and re-runs the macros.

fn main() {
    println!("cargo:rerun-if-changed=assets/themes");
    println!("cargo:rerun-if-changed=assets/sublime_themes");
    println!("cargo:rerun-if-changed=build.rs");
}
