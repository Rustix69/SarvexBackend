use std::{env, path::PathBuf};

fn main() {
    let protoc = protoc_bin_vendored::protoc_bin_path().expect("protoc binary");
    std::env::set_var("PROTOC", protoc);
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let proto_root = manifest_dir.join("../../../proto");
    let files = [
        "sarvex/v1/audit.proto",
        "sarvex/v1/common.proto",
        "sarvex/v1/ledger.proto",
        "sarvex/v1/marketdata.proto",
        "sarvex/v1/match.proto",
        "sarvex/v1/oracle.proto",
        "sarvex/v1/order.proto",
        "sarvex/v1/position.proto",
        "sarvex/v1/refdata.proto",
        "sarvex/v1/risk.proto",
        "sarvex/v1/rfq.proto",
        "sarvex/v1/settlement.proto",
    ];
    let paths: Vec<PathBuf> = files.iter().map(|file| proto_root.join(file)).collect();
    tonic_build::configure()
        .build_client(true)
        .build_server(true)
        .compile_protos(&paths, std::slice::from_ref(&proto_root))
        .expect("protobuf generation failed");
    println!("cargo:rerun-if-changed={}", proto_root.display());
}
