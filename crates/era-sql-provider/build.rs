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
                | "x86_64-pc-windows-msvc"
        ),
        "unsupported native SQLite target: {target}"
    );
    verify_windows_inputs(&target);
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
    verify_archive(&target);
}

fn verify_windows_inputs(target: &str) {
    if target == "x86_64-pc-windows-msvc" {
        assert!(
            !env::var("CARGO_CFG_TARGET_FEATURE")
                .unwrap_or_default()
                .split(',')
                .any(|value| value == "crt-static"),
            "native SQLite requires the dynamic MSVC CRT"
        );
        for name in [
            "LIB",
            "RUSTYERA_SQLITE_WINDOWS_INPUT_ROOTS",
            "CARGO_CFG_TARGET_FEATURE",
        ] {
            println!("cargo:rerun-if-env-changed={name}");
        }
        let roots = env::var_os("RUSTYERA_SQLITE_WINDOWS_INPUT_ROOTS")
            .expect("Windows input inventory required");
        for input in env::split_paths(&roots) {
            println!("cargo:rerun-if-changed={}", input.display());
        }
    }
}

fn verify_archive(target: &str) {
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
        library.join(if target == "x86_64-pc-windows-msvc" {
            "sqlite3.lib"
        } else {
            "libsqlite3.a"
        }),
        include.join("sqlite3.h"),
        library.join("manifest.json"),
    ] {
        println!("cargo:rerun-if-changed={}", file.display());
    }
    let verifier = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("crate directory"))
        .join("../../tools/sqlite-native/build.mjs");
    println!("cargo:rerun-if-changed={}", verifier.display());
    println!(
        "cargo:rerun-if-changed={}",
        verifier.with_file_name("windows.mjs").display()
    );
    let result = Command::new(env::var_os("RUSTYERA_NODE").unwrap_or_else(|| "node".into()))
        .arg(verifier)
        .arg("--verify-link-inputs")
        .arg(&library)
        .arg(target)
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
