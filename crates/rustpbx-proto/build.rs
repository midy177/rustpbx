fn main() -> Result<(), Box<dyn std::error::Error>> {
    let protoc = protoc_bin_vendored::protoc_bin_path().unwrap();
    // SAFETY: build script runs single-threaded before any spawned tasks.
    unsafe { std::env::set_var("PROTOC", protoc) };

    // Generate both server and client for every service. Client-only consumers
    // (edge/worker) simply don't reference the generated server types — the
    // extra code is harmless and keeps a single shared generation.
    tonic_build::configure()
        .build_server(true)
        .build_client(true)
        .compile_protos(
            &[
                "proto/control_plane.proto",
                "proto/edge_worker.proto",
                "proto/raft.proto",
            ],
            &["proto/"],
        )?;
    Ok(())
}
