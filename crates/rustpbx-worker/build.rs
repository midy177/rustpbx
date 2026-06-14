fn main() -> Result<(), Box<dyn std::error::Error>> {
    let protoc = protoc_bin_vendored::protoc_bin_path().unwrap();
    unsafe { std::env::set_var("PROTOC", protoc) };

    tonic_build::configure()
        .build_server(false)
        .build_client(true)
        .compile_protos(
            &["proto/control_plane.proto", "proto/edge_worker.proto"],
            &["proto/"],
        )?;
    Ok(())
}
