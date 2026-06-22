fn main() -> Result<(), Box<dyn std::error::Error>> {
    let protoc = protoc_bin_vendored::protoc_bin_path().unwrap();
    // SAFETY: build script runs single-threaded before any spawned tasks.
    unsafe { std::env::set_var("PROTOC", protoc) };

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
