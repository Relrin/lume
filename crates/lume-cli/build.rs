//! Build step for `lume`.
//!
//! On Windows the C runtime is linked into the program rather than loaded from a
//! library beside it. The command is shipped as a single file and must start on a
//! machine that has no Visual C++ runtime installed; the application's own build does
//! the same for the application.

fn main() {
    static_vcruntime::metabuild();
}
