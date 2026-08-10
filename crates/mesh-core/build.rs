fn main() {
    let protoc = protoc_bin_vendored::protoc_bin_path().expect("vendored protoc");
    // SAFETY: build script runs single-threaded before compilation.
    unsafe {
        std::env::set_var("PROTOC", protoc);
    }

    let mut config = prost_build::Config::new();
    config.bytes(["."]);
    config
        .compile_protos(&["proto/mesh/v1/control.proto"], &["proto"])
        .expect("compile control proto");
}
