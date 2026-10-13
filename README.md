# my-telemetry

Collects the telemetry of the services and shows it.

| Folder | |
|---|---|
| [my-telemetry-server](my-telemetry-server/) | Takes metrics over gRPC (`TelemetryWriter`) and HTTP (`POST /api/add`), keeps them by product scope and hour - on the local disk, then in S3 - gives them out over gRPC (`TelemetryReader`). There can be several. |
| [my-telemetry-ui](my-telemetry-ui/) | The UI: an HTTP server which serves the client and reads the my-telemetry-servers - one per env - over gRPC. |
| [my-telemetry-ui-client](my-telemetry-ui-client/) | The client of the UI: Dioxus, wasm. Built into `my-telemetry-ui/wwwroot/`. |
| [my-telemetry-ui-shared](my-telemetry-ui-shared/) | The models of the UI's `/ui/*` API, shared by the server and the client. |
| [proto-files](proto-files/) | The gRPC contracts. |

## my-telemetry-server

Settings - the YAML file `~/.my-telemetry` (without it - the YAML at `SETTINGS_URL`):

```yaml
DbPath: /root/db       # required: where the hours of metrics and the statistics are kept
HoursToGc: 48          # required: how long an hour is kept in S3; not used without S3
IgnoreEvents: []       # required: [{ name: <service>, data: <action> }] not to be kept
WindowSeconds: 5       # optional: how long a second of metrics waits in memory before it is written
# optional: where the hours go 2 hours after they end. Prefix is required - a folder of the
# bucket no other my-telemetry-server writes to. Debug=1 traces every request.
S3ConnString: "Endpoint=https://fsn1.your-objectstorage.com;Region=fsn1;AccessKey=...;SecretKey=...;Bucket=my-telemetry;Prefix=prod-1"
```

| Port | |
|---|---|
| `8000` (`HTTP_PORT`) | `POST /api/add` - writing metrics. Swagger. |
| `8888` (`GRPC_PORT`) | gRPC: `TelemetryWriter` - writing, `TelemetryReader` - reading, for my-telemetry-ui. |

### Product scopes

The metrics are kept and read by product scope. A writer names the scope of its metrics:
the gRPC metadata `scope` of `TelemetryWriter.Upload`, the header `scope` of
`POST /api/add` - a name of `a-z`, `0-9`, `-`, `_`, 64 characters at most. Metrics which
come with no scope are in the scope `default`. The UI shows the scopes there are metrics of
and reads the selected one; a trace by process id is read in all of them.

A service which writes with my-telemetry-writer names its scope by `product` of the
connection string its settings give: `url=http://my-telemetry:8888;product=my-product`.

### How long the metrics are kept

| | Without `S3ConnString` | With it |
|---|---|---|
| An hour is written into | until 2 hours after it ends - later metrics of it are dropped | the same |
| On the local disk | deleted 2 hours after it ends | uploaded to S3 then, and deleted |
| In S3 | - | kept for `HoursToGc` hours, then deleted |

With `HoursToGc` of 3 or less nothing is uploaded. What came in the last `WindowSeconds`
waits in memory: a clean stop writes it, a crash loses it. That time is also what the
`client_id` of a process has to come in - it comes with one of the events of the process,
and is put on the others as they are written.

### The files

```
<DbPath>/metrics/<scope>/<hour: YYYYMMDDHH>/
    metrics.data00 ...   the metrics, compressed
    second.index         where each second of the hour is
    key.index            which seconds have the events of each action of each service
    process.index        which seconds have the events of each process
<DbPath>/*.db            the statistics of the hours
```

In S3: `<Prefix>/<scope>/<hour>/` with the same files. The indexes are rebuilt from the
data files when they are lost or stale.

## my-telemetry-ui

Settings - the YAML file `~/.my-telemetry-ui`, read at start: without it the server does not start.

```yaml
envs: # the name shown in the UI -> gRPC of the my-telemetry-server of the env
  prod: http://10.0.0.3:8888
  stage: ssh:gateway@10.0.0.1:22->http://10.0.0.5:8888 # through an ssh tunnel

ssh_private_keys: # optional: without it the ssh agent is used
  "gateway@10.0.0.1:22": # or "*" - for every ssh host without a key of its own
    cert_path: /root/cert
    cert_pass_phrase: password # optional
```

It answers on `8000` (`HTTP_PORT`): the UI at `/`, its API at `/ui/*`, swagger at `/swagger`.

```bash
docker run -p 8000:8000 -v ~/.my-telemetry-ui:/root/.my-telemetry-ui ghcr.io/myjettools/my-telemetry-ui:<version>
```

## Build

Rust and `protoc` in `PATH`. Every crate builds on its own, from its folder:

```bash
cd my-telemetry-server && cargo build --release
cd my-telemetry-ui && cargo build --release
```

The client is built locally, after any change under `my-telemetry-ui-client/` or
`my-telemetry-ui-shared/`, and `my-telemetry-ui/wwwroot/` is committed - CI does not build it:

```bash
./my-telemetry-ui/build-ui.sh
```

It needs `dx` 0.7.10 (`cargo binstall dioxus-cli --version 0.7.10 --force`) and builds with
Rust 1.97, which it installs through rustup when it is missing.

## Release

A tag per project; the version is its tail:

| Tag | Image |
|---|---|
| `my-telemetry-server-<version>` | `ghcr.io/myjettools/my-telemetry-server:<version>` |
| `my-telemetry-ui-<version>` | `ghcr.io/myjettools/my-telemetry-ui:<version>` |
