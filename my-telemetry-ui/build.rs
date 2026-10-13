fn main() {
    // The reader side of my-telemetry-server's gRPC, shared from ../proto-files.
    ci_utils::ProtoFileBuilder::new("../proto-files/").sync_and_build("TelemetryReader.proto");
}
