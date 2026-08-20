# Architecture

Status: Draft
Last reviewed: 2026-08-20
Related capabilities: `ROUTE-*`, `OSC-*`, `REC-*`, `VMC-*`, `OPS-*`

## Responsibility boundary

`osc-toolkit-cli` owns the real-time data plane:

- UDP socket input and output
- N:N endpoint mappings
- Optional OSC-aware processing
- Recording and replay
- Validation, inspection, metrics, and machine-readable failures

It does not own:

- Task or workflow orchestration
- Process scheduling or supervision
- Configuration authoring/deployment
- Multi-instance coordination
- Web UI
- Domain-specific animation logic

The external orchestration engine starts, stops, restarts, and monitors CLI processes. V1 configuration is immutable during a process run.

## Runtime data flow

```text
UDP input
  -> input tap (optional recording)
  -> precompiled mapping edges
  -> optional OSC parse/filter/profile/remap
  -> mapping tap (optional recording)
  -> output queue/socket
  -> output tap (optional recording)
```

Key constraints:

- N:N is the public model; adjacency lists are only its compiled runtime representation.
- Passthrough packets are not decoded or re-encoded.
- Multiple inputs converging on one output are packet-level interleaving, not frame synchronization or application-level merging.
- All queues and pools are bounded and expose overflow counters.
- Packet logging is disabled by default and errors are rate-limited.

## Planned Rust workspace

The project should start as a workspace with narrow crates. This protects the raw forwarding and recording paths from profile or CLI dependencies.

```text
osc-toolkit-cli/
├── Cargo.toml
├── crates/
│   ├── osc-codec/          # Borrowed OSC views, validation, encoding
│   ├── osc-config/         # TOML model and structural validation
│   ├── osc-routing/        # N:N topology compilation, filters, remaps
│   ├── osc-recording/      # Binary format, writer, reader, recovery, replay cursor
│   ├── osc-profiles/       # Built-in spec profiles; initially VMC
│   ├── osc-runtime/        # UDP I/O, bounded queues, metrics, shutdown
│   └── osc-cli/            # Thin binary: argument parsing and command wiring
├── docs/
├── tests/
│   ├── fixtures/           # Versioned protocol/config/recording fixtures
│   └── integration/        # Cross-crate CLI and network behavior
├── benches/                # Reproducible forwarding/recording benchmarks
└── fuzz/                   # OSC and recording parsers once introduced
```

Do not add a crate solely for anticipated reuse. The listed boundaries are justified by independent invariants and testing needs; further splitting requires demonstrated value.

## Dependency direction

```text
osc-cli
  -> osc-runtime
  -> osc-config
  -> osc-routing -> osc-codec
  -> osc-recording
  -> osc-profiles -> osc-codec
```

Additional constraints:

- `osc-codec` knows nothing about VMC, routing, recording, runtime, or CLI.
- `osc-recording` stores datagrams and stream metadata; it does not depend on OSC or VMC semantics.
- `osc-profiles` may interpret borrowed OSC views but cannot own sockets, timers, or frame state.
- `osc-routing` owns generic endpoint mapping and generic OSC filter/remap semantics; profile-specific operations remain in `osc-profiles`.
- `osc-runtime` composes components but does not redefine their schemas.
- `osc-cli` contains no packet-processing logic.

Cycles are prohibited. A required reverse dependency indicates a boundary problem and must be resolved before implementation.

## Packet ownership

The target model is one owned receive buffer shared across fan-out destinations:

- Receive into a bounded/reused buffer.
- Passthrough mappings share the same immutable payload.
- Read-only OSC filtering uses borrowed views over the payload.
- A transformed payload is allocated only when a rule actually changes a packet.
- Recorder enqueue transfers or shares payload ownership without an additional copy where practical.

Exact buffer types and async runtime are Phase 0 benchmark decisions, not architectural promises.

## Configuration compilation

Startup is transactional:

1. Parse TOML.
2. Perform structural and semantic validation.
3. Resolve string IDs to compact runtime IDs.
4. Expand `all-to-all` and `zip` mapping groups.
5. Deduplicate or reject duplicate edges according to the configuration specification.
6. Compile filters, remaps, and profile selectors.
7. Bind all required sockets and open recorders.
8. Enter the running state only after the complete plan succeeds.

No partially active topology is allowed after a startup error.

## Profile model

Profiles are built-in, compile-time modules, not a dynamic plugin system. A profile may provide:

- Known address/schema recognition
- Argument validation
- Typed inspection
- Finite field selectors and exact-name remapping

A profile may not provide orchestration, timers, state aggregation, protocol conversion, or application logic. The initial profile is VMC; see [`profiles/vmc.md`](profiles/vmc.md).

## Recording boundary

Recording remains payload-agnostic:

```text
relative monotonic timestamp
stream/tap identity
optional source metadata
raw UDP datagram
```

OSC and VMC interpretation occurs during optional inspect/filter operations, not in the storage format. The normative binary layout will live in `specs/recording-format.md` before implementation.

## Platform boundary

Supported targets are Windows, macOS, and Linux on x86_64 and ARM64. Platform-specific socket tuning and kernel-drop metrics must live behind small adapters with explicit “unavailable” behavior rather than divergent core logic.
