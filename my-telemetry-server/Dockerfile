FROM ubuntu:22.04
# TLS to S3 is checked against the certificates of the system, and the image has none
RUN apt-get update && apt-get install -y ca-certificates
COPY ./target/release/my-telemetry-server ./my-telemetry-server
ENTRYPOINT ["./my-telemetry-server"]
