# Roadmap

Status: Active
Last reviewed: 2026-08-20

This document tracks delivery order and acceptance evidence. It does not duplicate normative architecture, configuration, recording-format, or profile specifications.

- Documentation ownership and anti-corruption rules: [`docs/README.md`](docs/README.md)
- System boundaries and planned repository structure: [`docs/architecture.md`](docs/architecture.md)
- Built-in VMC profile boundary: [`docs/profiles/vmc.md`](docs/profiles/vmc.md)

## Product boundary

`osc-toolkit-cli` is a cross-platform Rust data-plane process for high-throughput, bounded-memory UDP/OSC routing, filtering, remapping, recording, and replay.

The external orchestration engine owns process lifecycle, workflow execution, configuration deployment, and multi-instance coordination. This project does not implement a competing orchestration or task-management layer.

## Status vocabulary

| Status | Meaning |
|---|---|
| Planned | Scope is accepted but implementation has not started |
| Specifying | Normative specification or benchmark design is in progress |
| Implementing | Code is in progress |
| Verifying | Implementation exists; acceptance evidence is incomplete |
| Complete | Acceptance evidence is linked and all gates pass |
| Deferred | Explicitly outside the current release plan |

A capability may only move to **Complete** when its evidence column links to automated tests, benchmark results, fixtures, or release artifacts.

## Capability roadmap

### Foundation and contracts

| ID | Capability | Phase | Status | Acceptance gate | Evidence |
|---|---|---:|---|---|---|
| SPEC-01 | Versioned TOML configuration contract | 0 | Specifying | Normative schema defines IDs, endpoints, mappings, group expansion, defaults, errors, and versioning | [`docs/specs/configuration.md`](docs/specs/configuration.md) |
| SPEC-02 | Reproducible performance methodology | 0 | Planned | Workloads cover passthrough, fan-out, convergence, recording, bursts, and soak; hardware/runtime context is recorded | — |
| SPEC-03 | Binary recording format contract | 0 | Specifying | Versioned layout, recovery rules, limits, checksums, index, and compatibility fixtures are specified before writer code | [`docs/specs/recording-format.md`](docs/specs/recording-format.md) |
| FOUND-01 | Rust workspace and dependency boundaries | 0 | Implementing | Workspace follows [`docs/architecture.md`](docs/architecture.md); dependency graph is acyclic | [`Cargo.toml`](Cargo.toml) |
| FOUND-02 | Stable error and exit-code model | 0 | Planned | Human and machine-readable failures are deterministic and tested | — |

### N:N UDP data plane

| ID | Capability | Phase | Status | Acceptance gate | Evidence |
|---|---|---:|---|---|---|
| ROUTE-01 | UDP IPv4/IPv6 unicast inputs and outputs | 1 | Verifying | Bind/send integration tests pass on supported OS families | [`osc-runtime` tests](crates/osc-runtime/src/lib.rs), [live VMC smoke test](docs/evidence/2026-08-20-vmc-passthrough-smoke-test.md) |
| ROUTE-02 | Arbitrary N:N endpoint mappings | 1 | Verifying | Explicit fan-out, convergence, disconnected endpoints, and mixed topologies execute deterministically | [`osc-runtime` tests](crates/osc-runtime/src/lib.rs) |
| ROUTE-03 | Mapping groups: `all-to-all` and `zip` | 1 | Implementing | Expansion, length validation, duplicate policy, and ordering match `SPEC-01` | [`osc-config` tests](crates/osc-config/src/lib.rs) |
| ROUTE-04 | Zero-decode passthrough | 1 | Verifying | Raw payload is forwarded byte-identically without invoking OSC decode | [`forwards_payload_byte_identically`](crates/osc-runtime/src/lib.rs), [live VMC smoke test](docs/evidence/2026-08-20-vmc-passthrough-smoke-test.md) |
| ROUTE-05 | Transactional startup and graceful shutdown | 1 | Implementing | Invalid/partial startup exposes no running topology; shutdown drains according to documented bounds | [`osc-runtime` tests](crates/osc-runtime/src/lib.rs) |
| PERF-01 | Bounded packet buffers and queues | 1 | Planned | Burst and soak tests show bounded memory and observable overflow behavior | — |
| OBS-01 | Per-input, mapping, and output statistics | 1 | Planned | Counters and final summary are stable and machine-readable | — |
| REL-01 | Cross-platform CI artifacts | 1 | Planned | Windows/macOS/Linux x86_64 and ARM64 build matrix is green | — |

### Binary recording and replay

| ID | Capability | Phase | Status | Acceptance gate | Evidence |
|---|---|---:|---|---|---|
| REC-01 | Chunked raw-datagram writer | 2 | Implementing | Sequential writer follows `SPEC-03`, uses bounded memory, and passes format fixtures | [`osc-recording` tests](crates/osc-recording/src/lib.rs) |
| REC-02 | Input, mapping, and output recording taps | 2 | Implementing | Tap identity and pre/post-processing semantics are unambiguous and tested | [input tap runtime test](crates/osc-runtime/src/lib.rs) |
| REC-03 | Record-only and record-and-forward modes | 2 | Implementing | Both modes share writer semantics and expose loss/backpressure counters | [input tap runtime test](crates/osc-runtime/src/lib.rs) |
| REC-04 | CRC32C validation and crash recovery | 2 | Implementing | Forced termination recovers through the final complete chunk without requiring a footer | [`osc-recording` tests](crates/osc-recording/src/lib.rs) |
| REC-05 | Sparse index, inspect, verify, and repair | 2 | Planned | Large files can seek and rebuild indexes within documented resource bounds | — |
| REPLAY-01 | Timed and fastest-possible replay | 2 | Implementing | Order is exact and timing error is measured against an accepted tolerance | [CLI replay test](crates/osc-cli/tests/replay.rs) |
| REPLAY-02 | Speed, range, seek, loop, and stream selection | 2 | Planned | CLI integration tests cover combinations and invalid ranges | — |
| PERF-02 | Recorder backpressure policies | 2 | Planned | `drop-recording`, `block`, and `fail` behavior is bounded, documented, and stress-tested | — |

### Generic OSC processing

| ID | Capability | Phase | Status | Acceptance gate | Evidence |
|---|---|---:|---|---|---|
| OSC-01 | Safe OSC message/bundle codec | 3 | Planned | Messages, nested bundles, typetags, timetags, malformed input, and configured limits are tested/fuzzed | — |
| OSC-02 | Decode-on-demand and encode-on-change | 3 | Planned | Passthrough benchmark does not regress when OSC features are compiled in | — |
| OSC-03 | Address allow/deny filters | 3 | Planned | Exact, prefix, and OSC pattern behavior has conformance fixtures | — |
| OSC-04 | Address remapping | 3 | Planned | Exact and bounded pattern remaps preserve unaffected packet fields and bundle structure | — |
| OSC-05 | Generic OSC inspection | 3 | Planned | Stable text and machine-readable output handle known and unknown typetags per policy | — |

### Built-in VMC profile

Normative scope is in [`docs/profiles/vmc.md`](docs/profiles/vmc.md). VMC support is part of this repository and binary, but remains an optional OSC-aware path with no cost to raw passthrough beyond binary size.

| ID | Capability | Phase | Status | Acceptance gate | Evidence |
|---|---|---:|---|---|---|
| VMC-01 | Pinned VMC schema and validator | 3 | Planned | Upstream spec revision is pinned; supported messages and invalid boundaries have fixtures | — |
| VMC-02 | Typed VMC inspection | 3 | Planned | Known messages render named fields; unknown extensions fall back to generic OSC without loss | — |
| VMC-03 | Finite VMC selectors | 3 | Planned | Message category, bone, blendshape, and specified identifier filters are deterministic | — |
| VMC-04 | Exact VMC identifier remapping | 3 | Planned | Bone/blendshape/specified identifier names remap without numeric transform or unknown-message mutation | — |

### Production hardening

| ID | Capability | Phase | Status | Acceptance gate | Evidence |
|---|---|---:|---|---|---|
| HARD-01 | Parser fuzzing and malformed corpus | 4 | Planned | OSC and recording parsers run continuously against retained regression cases | — |
| HARD-02 | Burst, fan-out, convergence, disk-stall, and soak suites | 4 | Planned | Published benchmark report meets agreed latency, throughput, memory, and recovery thresholds | — |
| HARD-03 | Platform socket and file-I/O tuning | 4 | Planned | Platform differences are isolated, tested, and report unavailable metrics explicitly | — |
| OPS-01 | Generated CLI reference and stable JSON output | 4 | Planned | Generated docs match the binary; orchestration-facing output has compatibility tests | — |
| REL-02 | Signed/checksummed release process | 4 | Planned | Reproducible release workflow publishes supported artifacts and checksums | — |
| COMPAT-01 | Configuration and format compatibility policy | 4 | Planned | Upgrade/downgrade expectations and immutable released format versions are tested | — |

## Phase gates

### Phase 0 — Specifications and benchmark baseline

Complete only when:

- `SPEC-01`, `SPEC-02`, `SPEC-03`, `FOUND-01`, and `FOUND-02` are complete.
- Ambiguous mapping semantics and recording recovery behavior are resolved in normative documents.
- OSC codec options are benchmarked; custom codec work requires evidence that maintained crates cannot meet safety/allocation requirements.

### Phase 1 — N:N UDP data plane

Complete only when:

- Arbitrary N:N mappings work without OSC parsing.
- Long-running burst and soak tests demonstrate bounded memory.
- Transactional startup, partial socket failure, output failure, and shutdown are covered.
- Required cross-platform artifacts build successfully.

### Phase 2 — Recording and replay

Complete only when:

- Normal and forced termination files remain readable according to `SPEC-03`.
- Slow disk and full queues cannot cause unbounded memory growth.
- Loss is never hidden: application drops and available kernel drops are reported.
- Replay order, selection, seek, and timing have measurable acceptance results.

### Phase 3 — OSC and VMC awareness

Complete only when:

- Generic OSC features do not regress raw passthrough behavior.
- Parser limits and malformed input are hardened.
- VMC support stays within the spec-profile boundary and pins an upstream revision.
- VMC typed objects are not written into the generic recording container.

### Phase 4 — Production release

Complete only when:

- All supported platform builds and integration suites pass.
- Benchmark and soak evidence is published.
- Generated CLI reference, configuration version, recording format version, checksums, and release notes agree with the binary.

## Deferred until evidence of need

| ID | Capability | Status | Reason |
|---|---|---|---|
| FUT-01 | Runtime mapping-table replacement | Deferred | External orchestration plus restart is the initial lifecycle model |
| FUT-02 | Minimal orchestration-engine control adapter | Deferred | Must not duplicate task orchestration; define only from a concrete integration need |
| FUT-03 | UDP multicast | Deferred | Not part of core OSC semantics and no primary use case is established |
| FUT-04 | Recording compression codecs | Deferred | Requires measured storage benefit without unacceptable real-time cost |
| FUT-05 | Finite numeric argument transforms | Deferred | Add only for concrete, bounded OSC use cases |
| FUT-06 | Live Link Face binary to VMC OSC adapter | Deferred | Protocol conversion and motion logic are outside the VMC spec-profile boundary |

## Explicit non-goals

- Porting the old TypeScript architecture or recording format
- Task orchestration, workflow graphs, process supervision, or multi-instance scheduling
- Dynamic plugin runtime or profile scripting
- Web UI, HTTP control server, or WebSocket control plane
- Protocol-level frame synchronization, source priority, failover, or motion mixing
- Retargeting, coordinate conversion, quaternion conversion, blendshape arithmetic, or avatar logic
- Claims of guaranteed lossless UDP capture
- V1 multicast or TCP OSC

## Maintenance checklist

Every behavior-changing pull request must answer:

- Which capability IDs does it change?
- Does it modify a normative schema, format, profile, default, limit, exit code, or platform claim?
- Are fixtures/tests/benchmarks updated in the same pull request?
- Is completion evidence linked before changing roadmap status?
- Does generated CLI documentation still match the binary?

Roadmap reviews update `Last reviewed` even when no capability status changes. Detailed specifications must remain in their designated documents rather than being copied into this roadmap.
