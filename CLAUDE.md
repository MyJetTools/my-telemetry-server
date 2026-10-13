# my-telemetry

## The repository: a monorepo of three projects

The layout of my-prop-trading: a folder per crate, **no root cargo workspace** (each crate
builds on its own), shared crates by `path = "../…"`, shared protos in `proto-files/`, a
release workflow per project told apart by the tag.

```
proto-files/              TelemetryWriter.proto, TelemetryReader.proto
my-telemetry-server/      collects the telemetry and keeps it. There can be several.
my-telemetry-ui/          HTTP server: serves the client from wwwroot/, answers its /ui/*
                          by reading the my-telemetry-servers over gRPC (TelemetryReader)
my-telemetry-ui-client/   the client: Dioxus, wasm, client-side only
my-telemetry-ui-shared/   the /ui/* wire models, shared by my-telemetry-ui and the client
```

- **my-telemetry-server has no UI.** HTTP is for writing metrics (`POST /api/add`) and
  swagger; gRPC `TelemetryWriter` writes, `TelemetryReader` is what my-telemetry-ui reads.
  Anything the UI needs goes into `TelemetryReader.proto` first.
- **my-telemetry-ui reads several servers.** `envs` of its settings (`~/.my-telemetry-ui`)
  name them: `name -> gRPC url`, an `ssh:...->http://...` url for an ssh tunnel - the
  pattern of my-logger-ui. Every `/ui/*` request carries the `env` and - all but the trace
  by process id - the product `scope`; the client keeps the selected ones in localStorage,
  as it does the hour.
- **The client is built locally, never in CI**: `my-telemetry-ui/build-ui.sh` builds it
  with `dx` into `my-telemetry-ui/wwwroot/`, which is committed. `my-telemetry-ui-client/build.sh`
  pins Rust 1.97 for `dx` - since 1.98 `dx` 0.7 can not strip the release wasm on macOS
  (rust-lang/rust#164063).
- **Releases**: tag `my-telemetry-server-<v>` → `ghcr.io/myjettools/my-telemetry-server`,
  tag `my-telemetry-ui-<v>` → `ghcr.io/myjettools/my-telemetry-ui`. The server's series goes
  on from the tags of the repository before the monorepo (last: 0.4.0). The UI before it
  was the MyJetTools/my-telemetry-ui repository on Docker Hub (last: 0.2.3).

# my-telemetry-server

## What this service is

A **system** service, not a product one. The microservice architecture best practices
(`flows` → `scripts` → dedicated CRUD gRPC service, "public API has no DB", one level of
flows, and so on) **do not apply here**. Do not restructure this codebase to satisfy them.

The general Rust performance rules still apply — `parking_lot` over `tokio::sync` unless a
guard genuinely has to be held across `.await`, `AHash` for internal keys, no heavy CPU work
under a lock, no needless `.clone()` of payloads.

## Metrics storage — decisions already made

The hourly metric stream is kept in a hand-rolled storage, the one of my-logger-server
(`my-logger-server/src/logs_storage/`) copied and adapted - there is no shared library yet.
What follows is settled. Do not reopen these questions.

### Product scopes

- **The writer names the scope.** `product=xxxx` of its connection string arrives as the
  gRPC metadata `scope` of `TelemetryWriter.Upload` and the header `scope` of
  `POST /api/add` - the names my-logger-server reads. A name is trimmed and lower-cased:
  `a-z`, `0-9`, `-`, `_`, 64 characters at most. No scope, or a name which is not one -
  the metrics go to `default`.
- **The server has no scope settings**: no regular expressions by service name, no
  `settings.json`, no MCP. my-logger-server has them as a fallback; here there is none.
- **Everything is kept and read by scope**: the folders, the metrics waiting in memory, the
  statistics (the turso tables have `scope` in their keys), the list of hours, the events
  of an action. A reader which names no scope reads `default`.
- **A trace by process id is read in every scope of the hour**: a process goes through the
  services of more than one product.

### An hour is a folder

```
<DbPath>/metrics/<scope>/2026101207/
├── metrics.data00 … metrics.dataFF   32 KiB frames in one chain, a new file after 3 GiB
├── second.index                      3600 cells `file u8 · offset u32 · count u32` + a dirty byte
├── key.index                         records of each action of each service, by second
└── process.index                     pairs (process id, second): where the events of a process are
```

In S3 the same files: `<Prefix>/<scope>/2026101207/…`. The folder is named by the hour key
and nothing else, so parsing it back is `name.parse::<i64>()`, and deleting an hour is
deleting one folder. `frame.rs`, `cells.rs` and the file of `key.index` are my-logger-server's
byte for byte.

- **An entry of a frame** is `[varint length][time i64][zstd block]`: records of one second
  compressed together, 8 KiB compressed at most - a busy second takes several. Inside the
  block: `[varint length][TelemetryGrpcEvent]` each.
- **`second.index`**: the dirty byte is set before frames are written and cleared with the
  cells. Dirty, short or unreadable - the index is rebuilt by walking the chain.
- **`key.index`**: key = service (`name`), value = action (`data`), plus a reserved key with
  the totals of each service - exact counts for the one indexed search, "service + action".
  A service with more than 4096 actions in an hour is counted as a whole; `client_id` is
  not indexed.
- **`process.index`**: sorted 10-byte pairs, a directory of the first pair of each page of
  4096 at the head. **The second the process id itself names is not kept** - a process id
  is the unix-microsecond time its process started (`my-telemetry-core/src/ctx.rs`), so a
  trace always reads that second, and a process over within it takes nothing in the index.
  Kept are the other seconds: a process which runs on, a clock which is off, an id which
  is not a time. More than 4,000,000 pairs in an hour - the hour is not indexed by process.

### The record

The payload is `writer::TelemetryGrpcEvent` from
[proto-files/TelemetryWriter.proto](proto-files/TelemetryWriter.proto) - the message that
already arrives over the wire. There is deliberately **no separate storage proto**.

- **`TelemetryWriter.proto` field numbers are frozen.** It doubles as the on-disk format,
  and files outlive deploys. Fields may be added; numbers must never be reused and types
  must never change.
- **`client_id` lives as a tag, not as its own field** (`metric_tags::get` extracts it,
  `update_user_id_to_client_id` rewrites `user_id` into it).
- **A record which does not fit an entry is cut** (`metric_block::shrink`): the longest
  strings first, the tags last, `client_id` / `user_id` never.

### Baseline premise: events arrive in real time

99.95% of events arrive in real time, within millisecond intervals of their `started`.
Out-of-order arrival is **an infrastructure problem, not a case the format is designed
around**. Do **not** propose LSM trees, extent lists, per-second bucket structures or
anything similar. The rare path only has to be correct.

### Writing

1. **Taken in at once.** A writer's metrics go into `ToWriteQueue` and the writer signals
   `AppContext::queue_executor` - a `BackgroundExecutor` of rust-extensions. Its job
   (`flows::take_in_queued_metrics`) takes the queue without waiting for anything: counts
   the statistics, remembers which client each process is of, hands the metrics to the
   storage. No timer, no hold. The statistics count what is stored: a time which is
   whatever its writer gave is put at the time the metric came, a metric of an hour which
   is not written into any more is dropped here.
2. **One place to wait: the memory of the storage.** It holds the metrics by (scope,
   second) for `WindowSeconds` (5) from the first record of the second - `pending.rs`.
   What comes for the second within that is written with it, in one block.
3. **The client of a process is named as the second leaves that memory**
   (`flows::write_ready_metrics`, the `MetricsWriter` timer). An event is sent when it
   ends, so the one which carries the `client_id` - the request the others are a part of -
   is often the last to come: the window is the time it has to come in. Writing a metric
   to disk the moment it comes would leave the inner events of a process without their
   client - which is why the window stays, and why there is no queue by process with a
   hold of its own (`SecondsToFlush`) any more: the window subsumes it. Nothing is awaited
   between taking the seconds and handing them to the write, and what is kept for good
   (`permanent_metrics`) is kept after it: a tick which is given up on, or a table which
   does not take a row, does not cost the seconds.
4. `HourFile::write_second` puts the second into the chain. A second which has entries
   already gets the late records merged into its last block; a second older than what is
   written goes between the frames - a frame with room shifts its entries, a full one is
   halved. **The tail of a file is never rewritten.**

Records are compressed before the lock of the hour is taken. There is no `fsync`: the
protocol survives a crash of the process, not a power loss. A crash loses what waits in
memory - `WindowSeconds`; a clean shutdown takes in the queue, writes the storage out with
the indexes, and the statistics (`main.rs`). An hour whose time on the local disk is over
is not written into: its late records are dropped with a message.

### Indexes are derived data

`second.index` is rebuilt when it is dirty or does not read; `key.index` and
`process.index` are rebuilt - in one walk of the data files, the same one - when the file
does not count the records `second.index` does. **Losing an index is never data loss, and
a read must never fail because of one**: an index which says nothing makes the search scan.

Data which does not read under a clean `second.index` - what a power loss leaves, as
nothing is fsync'ed - is found by that walk, and the index is rebuilt as if it were dirty:
the chain is cut at an entry which is not one, a block which does not decompress counts
for nothing. The hour opens with what reads, and its indexes agree with it from then on.

`key.index` is written every 60 seconds while it changes. `process.index` grows with the
processes of the hour, not with its actions - written every minute it would be more bytes
than the metrics - so it is written once the hour has not been written into for 60
seconds, when the hour is sealed and at a clean shutdown; after a crash it is rebuilt in
the walk which rebuilds `key.index`. Both are filled in `write_second` - the place which
knows the second a record lands in; the events of a process do not come at once, so a
later one adds its second there.

### Reading

- **The last events of an action** (`search_action`): `key.index` says which seconds have
  them, `second.index` where they are; read from the end of the hour down, 100 at most.
- **A trace** (`search_process`): `process.index` of every scope of the hour names the
  seconds, and only they are read. Where it says nothing the hour is scanned from the
  second the process started in - a minute before it, for the clocks which are behind.
- **What waits in memory is found too, and there is no moment a metric is in neither.**
  The seconds taken to be written stay in memory until they are in the files
  (`PendingMetrics::being_written`); a search looks at memory before it reads the files,
  and a metric it then finds in both - written meanwhile - is told by its process, times,
  service and action, and given once. The tags are not compared: a metric gets its
  `client_id` as it is written - until then a search by client finds it only when the
  event itself named the client.
- A pass reads under the read lock of the hour and lets go of it: the writes into the hour
  wait for a pass, not for a search.

### Concurrency

- An hour is a `tokio::sync::RwLock<HourSlot>`: its guard is held across the `.await` of a
  file read, which is what `tokio::sync` is for. Everything else is `parking_lot`.
- The lock of the map of hours is not held while an hour is opened or rebuilt.
- What blocks - writes, `blocking_write`, listing folders, decompression of a search -
  runs in `spawn_blocking`.

### Retention and S3

| | Written into | On the local disk | In S3 |
|---|---|---|---|
| Without S3 | until 2 hours after the hour ends | deleted then | - |
| With S3 | until 2 hours after the hour ends | then sealed, copied compact, uploaded, deleted | until `HoursToGc` |

- `S3ConnString` of the settings, the shape of my-logger-server:
  `Endpoint=…;Region=…;AccessKey=…;SecretKey=…;Bucket=…;Prefix=…[;Debug=1]`. A string which
  does not read stops the start. **`Prefix` is required**: there are several servers, and
  two which shared a folder would overwrite and garbage-collect the hours of each other.
- Upload order: the data files, `key.index`, `process.index`, `second.index` the last - an
  hour is in S3 once its index is. An hour found there already is not uploaded again, its
  folder is deleted. A failed upload is tried again after 1, 2, 4 … 30 minutes, and goes
  on from the data files which are there already.
- Hours in S3 are read by ranged reads; their indexes are cached, the listing for a minute.
- S3 GC runs every 10 minutes. With `HoursToGc` of 3 or less nothing is uploaded.
- `HoursToGc` is not used without S3.

### Where the code lives

[my-telemetry-server/src/metrics_storage/](my-telemetry-server/src/metrics_storage/):

- `metrics_storage.rs` - the pool of hours by (scope, hour): push, flush, the two searches,
  the lists of scopes and hours, upload, GC. `AppContext::repo`.
- `hour_file.rs` - one hour: open and rebuild, `write_second`, flush, the compact copy.
- `frame.rs`, `cells.rs` - a frame and its entries; `second.index`.
- `metric_block.rs` - the only file which knows what a record is made of: blocks, cutting.
- `key_index.rs`, `process_index.rs` - the two indexes, whole and as a directory.
- `pending.rs` - what waits in memory. `query.rs`, `search.rs` - the two searches.
- `retention.rs` - when an hour is uploaded and deleted, as pure functions.
- `data_stream.rs` - reading a file of an hour from the local disk or from S3.
- `s3_archive.rs` - the hours in S3, behind `ObjectStore`; `MemoryStore` stands in for the
  bucket in the tests.

[my-telemetry-server/src/product_scope.rs](my-telemetry-server/src/product_scope.rs) - the name of a scope.

**turso is still a dependency** for `permanent_metrics`, `h_statistics` and
`h_app_statistics`. Those are aggregates and a per-client table, not the hourly metric
stream.

### Open questions

- **S3 has not been run against a real bucket.** The storage tests run on `MemoryStore`;
  `S3Bucket` - the adapter over `my-s3` - is covered by none. The first real upload is to
  be watched: `Debug=1` in `S3ConnString` traces every request.
- **A trace across the hour boundary** - `GetByProcessId` is asked for one hour; the part
  of a process which runs into the next one is found by asking for that hour.
- **Durability of what waits in memory** - `WindowSeconds` (5) are lost on a crash. Either
  accept it (it is telemetry), or write a raw append log alongside and replay it at start.
- **A shared library** for the storages of my-logger-server and my-telemetry-server - they
  are copies now.
