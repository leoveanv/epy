use std::path::Path;

fn compilation_unit(path: &Path) -> bool {
    path.extension().is_some_and(|extension| extension == "c")
        && !matches!(
            path.file_name().and_then(|name| name.to_str()),
            Some("pcre2_jit_match.c" | "pcre2_jit_misc.c" | "pcre2_ucptables.c")
        )
}

fn main() {
    println!("cargo:rerun-if-changed=upstream");
    let target = std::env::var("TARGET").expect("Cargo provides the compilation target");
    let upstream = Path::new("upstream");
    let mut builder = cc::Build::new();
    for (name, value) in [
        ("PCRE2_CODE_UNIT_WIDTH", "8"),
        ("PCRE2_STATIC", "1"),
        ("HAVE_STDLIB_H", "1"),
        ("HAVE_MEMMOVE", "1"),
        ("HAVE_CONFIG_H", "1"),
        ("STDC_HEADERS", "1"),
        ("SUPPORT_PCRE2_8", "1"),
        ("SUPPORT_UNICODE", "1"),
    ] {
        builder.define(name, value);
    }
    if target.contains("windows") {
        builder.define("HAVE_WINDOWS_H", "1");
    }
    for directory in ["src", "deps", "include"] {
        builder.include(upstream.join(directory));
    }
    for entry in std::fs::read_dir(upstream.join("src")).expect("The vendored engine sources exist")
    {
        let path = entry.expect("The vendored source entry is readable").path();
        if compilation_unit(&path) {
            builder.file(path);
        }
    }
    // #NOTE: Interpreter-only static linking keeps the backreference policy and cancellation checks active on every platform.
    builder.compile("ers_pcre2");
}
