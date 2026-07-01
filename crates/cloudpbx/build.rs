use chrono::Local;
use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let web = Path::new(&manifest_dir).join("web");
    ensure_web_dist(&web);

    println!(
        "cargo:rustc-env=CARGO_PKG_VERSION={}",
        env!("CARGO_PKG_VERSION")
    );

    let git_commit = get_git_commit_hash();
    println!("cargo:rustc-env=GIT_COMMIT_HASH={git_commit}");

    let git_branch = get_git_branch();
    println!("cargo:rustc-env=GIT_BRANCH={git_branch}");

    let git_dirty = get_git_dirty();
    println!("cargo:rustc-env=GIT_DIRTY={git_dirty}");

    let build_time = Local::now();
    println!(
        "cargo:rustc-env=BUILD_TIME_FMT={}",
        build_time.format("%Y-%m-%d %H:%M:%S %Z")
    );
    println!(
        "cargo:rustc-env=BUILD_DATE={}",
        build_time.format("%Y-%m-%d")
    );

    let commercial = if env::var_os("CARGO_FEATURE_COMMERCE").is_some() {
        "commerce"
    } else {
        "community"
    };
    let short_version = if git_dirty == "dirty" {
        format!(
            "{}-{}-dirty-{commercial}",
            env!("CARGO_PKG_VERSION"),
            git_commit
        )
    } else {
        format!("{}-{}-{commercial}", env!("CARGO_PKG_VERSION"), git_commit)
    };
    println!("cargo:rustc-env=SHORT_VERSION={short_version}");

    println!("cargo:rerun-if-changed=Cargo.toml");
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/index");
    println!("cargo:rerun-if-env-changed=CLOUDPBX_SKIP_WEB_BUILD");
}

fn ensure_web_dist(web: &Path) {
    let dist = web.join("dist");
    ensure_dist_stub(&dist);

    if !web.join("package.json").exists() {
        return;
    }
    if env::var_os("CARGO_FEATURE_BUILD_WEB").is_none() {
        return;
    }
    if env::var_os("CLOUDPBX_SKIP_WEB_BUILD").is_some() {
        println!("cargo:warning=CLOUDPBX_SKIP_WEB_BUILD set; skipping CloudPBX web build");
        return;
    }

    println!(
        "cargo:rerun-if-changed={}",
        web.join("package.json").display()
    );
    println!("cargo:rerun-if-changed={}", web.join("bun.lock").display());
    println!(
        "cargo:rerun-if-changed={}",
        web.join("index.html").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        web.join("vite.config.ts").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        web.join("tsconfig.json").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        web.join("components.json").display()
    );
    rerun_if_changed_dir(&web.join("src"));

    let pm = if has_cmd("bun") {
        "bun"
    } else if has_cmd("npm") {
        "npm"
    } else {
        println!(
            "cargo:warning=neither `bun` nor `npm` found on PATH; embedding CloudPBX web stub"
        );
        return;
    };

    if !web.join("node_modules").exists() {
        println!("cargo:warning=installing CloudPBX web dependencies with `{pm} install`");
        run(pm, &["install"], web);
    }

    run(pm, &["run", "build"], web);
}

fn ensure_dist_stub(dist: &Path) {
    let index = dist.join("index.html");
    if index.exists() {
        return;
    }
    if let Err(e) = std::fs::create_dir_all(dist) {
        println!("cargo:warning=could not create {}: {e}", dist.display());
        return;
    }
    let stub = "<!doctype html><html><head><meta charset=\"utf-8\">\
        <title>CloudPBX</title></head><body>\
        <p>CloudPBX web console was not built. Run `bun run build` in \
        crates/cloudpbx/web or build without CLOUDPBX_SKIP_WEB_BUILD.</p>\
        </body></html>";
    if let Err(e) = std::fs::write(&index, stub) {
        println!("cargo:warning=could not write {}: {e}", index.display());
    }
}

fn rerun_if_changed_dir(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
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

fn has_cmd(cmd: &str) -> bool {
    Command::new(cmd)
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn run(pm: &str, args: &[&str], cwd: &Path) {
    let status = Command::new(pm)
        .args(args)
        .current_dir(cwd)
        .status()
        .unwrap_or_else(|e| panic!("failed to spawn `{pm} {}`: {e}", args.join(" ")));
    if !status.success() {
        panic!(
            "`{pm} {}` failed ({status}). Fix the CloudPBX web build, or set \
             CLOUDPBX_SKIP_WEB_BUILD=1 / build with --no-default-features to skip it.",
            args.join(" ")
        );
    }
}

fn get_git_commit_hash() -> String {
    let commit_from_git = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .unwrap_or_else(|_| "unknown".to_string());

    if commit_from_git == "unknown" {
        env::var("GIT_COMMIT_HASH").unwrap_or_else(|_| "unknown".to_string())
    } else {
        commit_from_git
    }
}

fn get_git_branch() -> String {
    let branch_from_git = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .unwrap_or_else(|_| "unknown".to_string());

    if branch_from_git == "unknown" {
        env::var("GIT_BRANCH").unwrap_or_else(|_| "unknown".to_string())
    } else {
        branch_from_git
    }
}

fn get_git_dirty() -> String {
    let dirty_from_git = Command::new("git")
        .args(["diff", "--quiet", "--ignore-submodules"])
        .output()
        .map(|output| {
            if output.status.success() {
                "clean"
            } else {
                "dirty"
            }
        })
        .unwrap_or_else(|_| "unknown")
        .to_string();

    if dirty_from_git == "unknown" || dirty_from_git == "dirty" {
        env::var("GIT_DIRTY").unwrap_or(dirty_from_git)
    } else {
        dirty_from_git
    }
}
