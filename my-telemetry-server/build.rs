fn main() {
    // The protos are shared with my-telemetry-ui and live in ../proto-files.
    ci_utils::ProtoFileBuilder::new("../proto-files/")
        .sync_and_build("TelemetryWriter.proto")
        .sync_and_build("TelemetryReader.proto");
}
