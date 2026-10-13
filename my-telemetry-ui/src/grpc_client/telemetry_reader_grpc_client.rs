use my_grpc_extensions::client::*;

// The timeout is for the slowest read: GetByProcessId scans the whole hour.
#[generate_grpc_client(
    proto_file: "../proto-files/TelemetryReader.proto",
    crate_ns: "crate::reader_grpc",
    retries: 3,
    request_timeout_sec: 60,
    ping_timeout_sec: 1,
    ping_interval_sec: 3,
)]
pub struct TelemetryReaderGrpcClient;
