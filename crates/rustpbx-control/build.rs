//! Build script: compile the Vue admin SPA (`web/`) into `web/dist` so the
//! control binary's static file server has something to serve after a plain
//! `cargo build -p rustpbx-control`.
//!
//! Behaviour:
//!   - Gated on the `build-web` feature (in `default`). Disable with
//!     `--no-default-features` or set `RUSTPBX_SKIP_WEB_BUILD=1` to skip.
//!   - Prefers `bun`, falls back to `npm`. If neither is on PATH, it warns and
//!     skips rather than failing — so backend-only environments still build.
//!   - Runs `<pm> install` only when `node_modules` is missing, then
//!     `<pm> run build`.
//!   - Re-runs only when web sources/config change (so normal Rust edits don't
//!     trigger a JS rebuild).

use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let web = Path::new(&manifest_dir).join("web");

    // The build script itself only needs to re-run when the web project changes
    // (or this script does). Without a web project, there's nothing to do.
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=RUSTPBX_SKIP_WEB_BUILD");
    if !web.join("package.json").exists() {
        return;
    }

    // Opt-outs: the cargo feature, or an env escape hatch (e.g. for rust-analyzer
    // or fast backend iteration).
    if std::env::var_os("CARGO_FEATURE_BUILD_WEB").is_none() {
        return;
    }
    if std::env::var_os("RUSTPBX_SKIP_WEB_BUILD").is_some() {
        println!("cargo:warning=RUSTPBX_SKIP_WEB_BUILD set — skipping web SPA build");
        return;
    }

    // Re-run when any web source or key config file changes.
    for f in [
        "package.json",
        "bun.lock",
        "index.html",
        "vite.config.ts",
        "tsconfig.json",
        "components.json",
    ] {
        let p = web.join(f);
        if p.exists() {
            println!("cargo:rerun-if-changed={}", p.display());
        }
    }
    rerun_if_changed_dir(&web.join("src"));

    // Choose a package manager: prefer bun, fall back to npm.
    let pm = if has_cmd("bun") {
        "bun"
    } else if has_cmd("npm") {
        "npm"
    } else {
        println!(
            "cargo:warning=neither `bun` nor `npm` found on PATH — skipping web SPA build. \
             Install bun (https://bun.sh) or provide a prebuilt web/dist."
        );
        return;
    };

    // Install dependencies on first build.
    if !web.join("node_modules").exists() {
        println!("cargo:warning=installing web dependencies with `{pm} install` (first build)…");
        run(pm, &["install"], &web);
    }

    // Build the SPA. A failure here fails the cargo build — a broken frontend
    // shouldn't ship silently.
    run(pm, &["run", "build"], &web);
}

/// Recursively emit `rerun-if-changed` for every file under `dir`.
fn rerun_if_changed_dir(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    // Emit the dir itself so added/removed children are caught too.
    println!("cargo:rerun-if-changed={}", dir.display());
    for entry in entries.flatten() {
        let path: PathBuf = entry.path();
        if path.is_dir() {
            rerun_if_changed_dir(&path);
        } else {
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }
}

/// Whether `cmd` resolves on PATH (cheap `--version` probe).
fn has_cmd(cmd: &str) -> bool {
    Command::new(cmd)
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Run `pm args…` in `cwd`, panicking (and thus failing the build) on error.
fn run(pm: &str, args: &[&str], cwd: &Path) {
    let status = Command::new(pm)
        .args(args)
        .current_dir(cwd)
        .status()
        .unwrap_or_else(|e| panic!("failed to spawn `{pm} {}`: {e}", args.join(" ")));
    if !status.success() {
        panic!(
            "`{pm} {}` failed ({status}). Fix the web build, or set \
             RUSTPBX_SKIP_WEB_BUILD=1 / build with --no-default-features to skip it.",
            args.join(" ")
        );
    }
}
