# CloudPBX

This crate contains the copied monolith PBX implementation used for the
CloudPBX refactor. It is kept outside the root Cargo workspace and builds as an
independent package.

## Running Locally

From this directory:

```bash
cargo run --bin cloudpbx -- --conf rustpbx.toml
```

From the repository root:

```bash
cargo run --manifest-path crates/cloudpbx/Cargo.toml --bin cloudpbx -- --conf crates/cloudpbx/rustpbx.toml
```

The SIP flow helper binary is available as:

```bash
cargo run --manifest-path crates/cloudpbx/Cargo.toml --bin cloudpbx-sipflow -- --help
```

## Build and Check

Use the crate manifest directly because `crates/cloudpbx` is not a root
workspace member:

```bash
cargo check --manifest-path crates/cloudpbx/Cargo.toml --bin cloudpbx
cargo build --manifest-path crates/cloudpbx/Cargo.toml --release --bin cloudpbx
```

`build.rs` injects version metadata required by `src/version.rs`, including
`GIT_COMMIT_HASH`, `GIT_BRANCH`, `GIT_DIRTY`, `BUILD_TIME_FMT`, `BUILD_DATE`,
and `SHORT_VERSION`.

## Frontend Packaging

CloudPBX currently serves the Vue frontend from `web/dist` at runtime. Build it
before running packaged binaries:

```bash
cd ../../web
bun install
bun run build
```

At the moment the frontend is not embedded into the `cloudpbx` binary. To make
the release self-contained, add a `build-web` feature, run the web build from
`crates/cloudpbx/build.rs`, add `rust-embed`, and replace `ServeDir::new("web/dist")`
in `src/app.rs` with embedded asset serving and SPA fallback to `index.html`.
