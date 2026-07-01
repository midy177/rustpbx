# CloudPBX

This crate contains the copied monolith PBX implementation used for the
CloudPBX refactor. It is kept outside the root Cargo workspace and builds as an
independent package.

## Running Locally

From this directory:

```bash
cargo run --bin cloudpbx -- --conf ../../config.toml.example
```

From the repository root:

```bash
cargo run --manifest-path crates/cloudpbx/Cargo.toml --bin cloudpbx -- --conf config.toml.example
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

CloudPBX has its own Vue frontend under `crates/cloudpbx/web`. The default
Cargo build runs the frontend build and embeds `web/dist` into the binary with
`rust-embed`, so release artifacts do not need an external `web/dist` folder.

Run the frontend directly during UI development:

```bash
cd crates/cloudpbx/web
bun install
bun run dev
```

Build the frontend manually:

```bash
cd crates/cloudpbx/web
bun run build
```

Skip the frontend build for backend-only iteration:

```bash
CLOUDPBX_SKIP_WEB_BUILD=1 cargo check --manifest-path crates/cloudpbx/Cargo.toml --bin cloudpbx
```

When skipped, `build.rs` embeds a small placeholder page if `web/dist/index.html`
does not already exist.
