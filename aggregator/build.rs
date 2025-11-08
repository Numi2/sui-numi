// Build script for ultra-aggr
// This file contains custom build instructions and code generation logic
// that runs during the compilation process
//
// Numan Thabit 2025 Nov

use std::{env, path::PathBuf};

fn main() {
    let apis_dir = env::var("SUI_APIS_DIR")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("vendor/sui-apis"));

    let proto_root = apis_dir.join("proto").join("sui").join("rpc").join("v2");

    if !proto_root.exists() {
        println!(
            "cargo:warning=Skipping gRPC proto generation; {:?} not found. \
             Set SUI_APIS_DIR to a checkout of mystenlabs/sui for full gRPC support.",
            proto_root
        );
        return;
    }

    let mut protos = vec![
        proto_root.join("bcs.proto"),
        proto_root.join("argument.proto"),
        proto_root.join("executed_transaction.proto"),
        proto_root.join("ledger_service.proto"),
        proto_root.join("move_package_service.proto"),
        proto_root.join("name_service.proto"),
        proto_root.join("signature.proto"),
        proto_root.join("signature_verification_service.proto"),
        proto_root.join("state_service.proto"),
        proto_root.join("subscription_service.proto"),
        proto_root.join("transaction_execution_service.proto"),
    ];

    let google_root = apis_dir.join("proto").join("google");

    protos.extend([
        google_root.join("rpc").join("status.proto"),
        google_root.join("rpc").join("error_details.proto"),
        google_root.join("protobuf").join("any.proto"),
        google_root.join("protobuf").join("duration.proto"),
        google_root.join("protobuf").join("empty.proto"),
        google_root.join("protobuf").join("field_mask.proto"),
        google_root.join("protobuf").join("struct.proto"),
        google_root.join("protobuf").join("timestamp.proto"),
    ]);

    tonic_build::configure()
        .build_server(false)
        .protoc_arg("--experimental_allow_proto3_optional")
        .compile(
            &protos
                .iter()
                .map(|p| p.to_string_lossy().to_string())
                .collect::<Vec<_>>(),
            &[
                apis_dir.join("proto"),
                apis_dir.join("proto").join("google"),
            ],
        )
        .expect("failed to compile Sui gRPC protos");
}
