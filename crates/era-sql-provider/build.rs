use std::{env, fs, path::PathBuf, process::Command};

fn main() {
    // Never introduce a provider-local test escape hatch around the native contract.
    // Transitive dependency features are not exported to this build script; their
    // actual linked engine is independently rejected by identity::verify().
    for name in [
        "CARGO_FEATURE_SYSBUNDLED",
        "CARGO_FEATURE_BUNDLED",
        "CARGO_FEATURE_BUNDLED_WINDOWS",
        "CARGO_FEATURE_SQLCIPHER",
        "CARGO_FEATURE_BUNDLED_SQLCIPHER",
        "CARGO_FEATURE_LOADABLE_EXTENSION",
    ] {
        println!("cargo:rerun-if-env-changed={name}");
        assert!(
            env::var_os(name).is_none(),
            "incompatible native SQLite provider feature: {name}"
        );
    }
    for name in [
        "SQLITE3_LIB_DIR",
        "SQLITE3_INCLUDE_DIR",
        "SQLITE3_STATIC",
        "SQLITE3_NO_PKG_CONFIG",
        "LIBSQLITE3_SYS_USE_PKG_CONFIG",
        "RUSTYERA_NODE",
    ] {
        println!("cargo:rerun-if-env-changed={name}");
    }
    let target = env::var("TARGET").expect("TARGET");
    assert_eq!(
        env::var("HOST").expect("HOST"),
        target,
        "native SQLite cross compilation is unsupported"
    );
    assert!(
        matches!(
            target.as_str(),
            "aarch64-apple-darwin"
                | "x86_64-apple-darwin"
                | "aarch64-unknown-linux-gnu"
                | "x86_64-unknown-linux-gnu"
        ),
        "unsupported native SQLite target: {target}"
    );
    for name in ["SQLITE3_STATIC", "SQLITE3_NO_PKG_CONFIG"] {
        assert_eq!(
            env::var(name).as_deref(),
            Ok("1"),
            "{name} must be 1 before Cargo starts"
        );
    }
    assert!(
        env::var_os("LIBSQLITE3_SYS_USE_PKG_CONFIG").is_none_or(|value| value == "0"),
        "LIBSQLITE3_SYS_USE_PKG_CONFIG must not override the native SQLite contract"
    );
    let library = PathBuf::from(env::var_os("SQLITE3_LIB_DIR").expect("SQLite prebuild required"));
    let include = PathBuf::from(
        env::var_os("SQLITE3_INCLUDE_DIR").expect("SQLite include directory required"),
    );
    assert_eq!(
        fs::canonicalize(&library).expect("library directory"),
        fs::canonicalize(&include).expect("include directory"),
        "SQLite archive and header must come from the same prebuild"
    );
    for file in [
        library.join("libsqlite3.a"),
        include.join("sqlite3.h"),
        library.join("manifest.json"),
    ] {
        println!("cargo:rerun-if-changed={}", file.display());
    }
    let verifier = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("crate directory"))
        .join("../../tools/sqlite-native/build.mjs");
    println!("cargo:rerun-if-changed={}", verifier.display());
    let result = Command::new(env::var_os("RUSTYERA_NODE").unwrap_or_else(|| "node".into()))
        .arg(verifier)
        .arg("--verify-link-inputs")
        .arg(&library)
        .arg(&target)
        .output()
        .expect("existing Node executable required to verify native SQLite inputs");
    assert!(
        result.status.success(),
        "SQLite link verification failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let digest = String::from_utf8(result.stdout).expect("link digest UTF-8");
    let digest = digest.trim();
    assert!(
        digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "invalid SQLite link digest"
    );
    // identity.rs consumes this value, changing Rust metadata even at an unchanged LIB_DIR.
    println!("cargo:rustc-env=RUSTYERA_SQLITE_LINK_IDENTITY={digest}");
    if target.ends_with("-linux-gnu") {
        println!("cargo:rustc-link-lib=m");
        println!("cargo:rustc-link-lib=pthread");
    }
}
