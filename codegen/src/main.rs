//! Writes the SDK's generated gRPC code: the server stubs the SDK serves
//! (`src/pb/apps.v0.rs`) and the client stubs its tests call it with, as
//! Nebo does (`tests/pb/apps.v0.rs`).

const PROTOS: &[&str] = &[
    "proto/apps/v0/common.proto",
    "proto/apps/v0/tool.proto",
    "proto/apps/v0/channel.proto",
    "proto/apps/v0/gateway.proto",
    "proto/apps/v0/comm.proto",
    "proto/apps/v0/ui.proto",
    "proto/apps/v0/schedule.proto",
];

/// The services the tests call. (A client for a service with a `Connect`
/// rpc does not compile: it collides with the generated `connect`.)
const TEST_CLIENT_PROTOS: &[&str] = &["proto/apps/v0/common.proto", "proto/apps/v0/tool.proto", "proto/apps/v0/ui.proto"];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    for (dir, protos, server) in [("src/pb", PROTOS, true), ("tests/pb", TEST_CLIENT_PROTOS, false)] {
        let protos: Vec<_> = protos.iter().map(|p| root.join(p)).collect();
        tonic_build::configure()
            .build_server(server)
            .build_client(!server)
            .out_dir(root.join(dir))
            .compile_protos(&protos, &[&root])?;
    }
    Ok(())
}
