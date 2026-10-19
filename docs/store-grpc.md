# Database transport

`OLDIF_DB_GRPC_URL` selects the private database gRPC service. Leaving it unset
or empty selects the existing HTTP transport configured by `OLDIF_DB_URL`.
A gRPC error is returned to the caller; it never triggers an HTTP fallback.
The database enables its listener only when `DB_GRPC_BIND` is configured.

The independently copied `proto/store.proto` contract has typed owner, kind,
identifier, revision and timestamp fields. Record content remains JSON bytes
because the existing database stores heterogeneous records. `Get`, streamed
`List` and atomic `Write` use the same database Store as the HTTP API, including
owner validation, immutable originals, revision checks and batch rollback.
The existing internal bearer token is required on every RPC.

Connection setup defaults to five seconds. The complete operation, including
all streamed list frames, defaults to 120 seconds. Both are configurable within
1–120 seconds. Requests are capped at 4 MB, individual records at 2 MB, and list
responses at 10,000 records and 64 MB of JSON content. Invalid, duplicated,
foreign-owner or incomplete responses return an error rather than partial data.

Run the actual compiled-service test after building this repository and the
independent database repository:

```sh
python3 scripts/check_store_grpc.py \
  --back-binary target/debug/oldif-back \
  --db-binary ../oldif-back-db/target/debug/oldif-back-db \
  --output evidence/store-grpc-validation.json
```

The test creates temporary SQLite storage, an ephemeral internal token and
loopback listeners. The backend's HTTP database URL is deliberately unavailable;
the observer uses database HTTP to verify transport parity. No model endpoint is
reachable. Synthetic sessions and the bundled public source collections are
used, and both spawned processes and the temporary storage are removed on exit.
