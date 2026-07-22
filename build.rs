//! Compile the vendored Handy RPC `.proto` files with `prost-build`.
//!
//! The four files come from `handy-public-rpc` (package `hdy_rpc`). They are
//! compiled into a single Rust module emitted to `$OUT_DIR/hdy_rpc.rs`, which
//! `src/rpc/mod.rs` then `include!`s.

fn main() -> std::io::Result<()> {
    let protos = [
        "proto/handy_rpc.proto",
        "proto/messages.proto",
        "proto/notifications.proto",
        "proto/constants.proto",
    ];

    for p in &protos {
        println!("cargo:rerun-if-changed={p}");
    }

    prost_build::compile_protos(&protos, &["proto/"])
}
