# my-telemetry-server

## What this service is

A **system** service, not a product one. The microservice architecture best practices
(`flows` → `scripts` → dedicated CRUD gRPC service, "public API has no DB", one level of
flows, and so on) **do not apply here**. Do not restructure this codebase to satisfy them.

The general Rust performance rules still apply — `parking_lot` over `tokio::sync` unless a
guard genuinely has to be held across `.await`, `AHash` for internal keys, no heavy CPU work
under a lock, no needless `.clone()` of payloads.

## Metrics storage format — decisions already made

The per-hour turso/sqlite files are being replaced with a hand-rolled binary format.
What follows is settled. Do not reopen these questions.

### What is being replaced

Today each hour is its own `<prefix>-<hourKey>.db` (turso/sqlite) with a `metrics` table:
`id, started, duration_micro, name, data, success, fail, tags json, client_id`,
`PRIMARY KEY (name, data, started)`, inserted through `INSERT OR IGNORE`.
`id` is `ProcessId` from `TelemetryWriter.proto` — a correlation id, **not unique**.
See [src/db/metrics/](src/db/metrics/), [src/db/metrics/hour_db_pool.rs](src/db/metrics/hour_db_pool.rs).

### Target: a folder per hour

An hour is a **folder under the db path, named by its hour key and nothing else**:

```
<DbPath>/
├── 2026010512/
│   ├── metrics.data     <- the records
│   └── second.index     <- 3600 slots, second of the hour -> offset into metrics.data
├── 2026010513/
│   └── ...
```

The name is the key, so parsing it back is `name.parse::<i64>()`. Deleting an hour is
deleting one folder — there is no way to leave half of it behind, and a later index on
another axis (e.g. by `ProcessId`) drops in next to `second.index` with no naming scheme
to renegotiate.

**Data file** — a flat log of records:

```
[payload length: u16][payload: protobuf]
[payload length: u16][payload: protobuf]
...
```

Append-only, and sorted by time **by construction** (see the one-minute window below).

The payload is `writer::TelemetryGrpcEvent` from [proto/TelemetryWriter.proto](proto/TelemetryWriter.proto) —
the message that already arrives over the wire. There is deliberately **no separate storage
proto**: it would be a near-duplicate message plus a build step, buying only hypothetical
schema independence that protobuf's own compatibility rules already provide.

Two consequences of that reuse, both accepted:

- **`TelemetryWriter.proto` field numbers are now frozen.** It doubles as the on-disk format,
  and files outlive deploys (they live `HoursToGc` hours). Fields may be added; numbers must
  never be reused and types must never change.
- **`client_id` lives as a tag, not as its own field** — which is its original form anyway
  (`metric_tags::get` extracts it from tags, `update_user_id_to_client_id` rewrites `user_id`
  into it). The sqlite `client_id` column disappears with the table.

**Second index file** — 3600 slots, one per second of the hour.
Slot N holds the offset into the data file where second N begins.
Reading "give me events from second N" is one seek plus a forward scan.

### Baseline premise: events arrive in real time

99.95% of events arrive in real time, within millisecond intervals of their `started`.
Out-of-order arrival is **an infrastructure problem, not a case the format is designed around**.
Even a one-minute lateness handled by an expensive path is cheaper than running a whole DBMS.

Consequence for design: do **not** propose LSM trees, extent lists, per-second bucket structures
or anything similar just to avoid the rare insert. The rare path only has to be correct.

### The one-minute in-memory window

The last ~60 seconds of events are held in memory, sorted.
Out-of-order arrivals are absorbed **in memory** (a shift inside a vector — cheap).
Only records older than a minute go to disk, and they go as an already-ordered tail.

This is why the data file is append-only by construction rather than by luck.

The window subsumes the current `ToWriteQueue` ([src/to_write_queue/](src/to_write_queue/)),
which today holds events in `AHashMap<process_id, MetricsChunkByProcessId>` with
`seconds_to_flush = 3` for the same reason — but groups by `process_id` rather than by time,
so records reach disk interleaved.

### Late beyond the window: the heavy insert

When an event is later than the window, the cost is known exactly, from the index:

- data: rewrite from `index[second]` to EOF
- index: rewritten whole, always — 14.4Kb is not worth the arithmetic of a partial write,
  and a whole rewrite cannot leave the index half-shifted

That is a concrete number — measurable, loggable, and expressible as a setting
("how many bytes we are willing to rewrite") — rather than an opaque mid-file insert.
The tail size is logged on every heavy insert.

Algorithm:

1. Use the index to find the offset of the target second; stream the file tail from disk
   into an intermediate buffer.
2. Locate the insertion point in the buffer and insert the record.
3. The offset delta is the record size (`2 + len(payload)`) — it is **constant** for every
   later index slot. Nothing is recomputed; the slots are just incremented.
4. Write the data tail back and rewrite the index.

What makes a record an append rather than an insert is `last_written_second`, not the
index: the index cannot distinguish "a later record in the second already on disk" from
"a record that belongs before it".

### Concurrency

The index and the data file live under a shared `tokio::Mutex`, so the heavy insert is
transactional: a reader can never observe a state where the data has shifted but the index
has not.

It follows that readers of that hour block for the duration of a heavy insert. That is a
deliberate trade: the path is rare (see the real-time premise), and the alternative — an
RCU-style copy — does not pay for itself.

### Where the code lives

[src/storage_by_hour/](src/storage_by_hour/) — saving and searching metrics.

- `storage_by_hour.rs` — one hour. The wrapper: owns the `tokio::Mutex` and nothing else.
- `storage_by_hour_inner.rs` — every decision: window, flush, append, heavy insert, scan,
  index load and rebuild, and the folder/file naming.
- `metrics_storage.rs` — the pool of open hours. Replaced the turso `HourDbPool`; the field
  is still `AppContext::repo`.
- `second_index.rs` — the 3600-slot index.
- `metric_record.rs` — `[u16 len][payload]` codec and the record iterator.

`MetricsRepo`, `HourDbPool` and the `metrics` table are gone. `MetricDto` stayed — it is
the in-memory model everywhere — but its DDL, columns and row mapping went with the table.

**turso is still a dependency** for `permanent_metrics`, `h_statistics` and
`h_app_statistics`. Those are aggregates and a per-client table, not the hourly metric
stream, and they were not part of this swap.

The index is **derived data**: a missing, short or torn index is rebuilt by walking the
data file, and a torn tail is truncated back to the last whole record. Losing the index
is therefore never data loss, and reads must never fail because of it.

### Open questions

- **`u16` payload length** — a hard 64 KB ceiling per record. `Fail` is a string and a client
  can put a stack trace in it. Decide: truncate on overflow, or spend 4 bytes on the length.
- **`GetByProcessId`** — the second index does not serve this axis at all. Either a full scan
  of the hour, or a second index keyed by `ProcessId`.
- **Durability of the in-memory window** — the minute in memory is lost on a crash (today the
  turso `INSERT` has already committed it). Either accept the loss (it is telemetry), or write
  a raw append log alongside and replay it at startup — one sequential write, no seeks.
- **Atomicity of the heavy insert** — `tokio::Mutex` rules out the reader/writer race, but not
  a crash midway through rewriting the tail, which leaves the file corrupt. Decide whether this
  needs a temp file plus `rename`, or whether failure on that rare path is acceptable.
