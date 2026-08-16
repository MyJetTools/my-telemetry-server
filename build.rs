fn main() {
    ci_utils::compile_protos("proto/TelemetryWriter.proto");
    ci_utils::compile_protos("proto/TelemetryReader.proto");
}
