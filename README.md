# osc-toolkit-cli

A cross-platform Rust CLI for high-throughput, bounded-memory UDP/OSC routing, filtering, remapping, recording, and replay.

The project is a **data-plane tool**. Task orchestration, process lifecycle management, configuration deployment, and multi-instance coordination belong to an external orchestration engine.

## Planned capabilities

- Arbitrary N:N UDP input/output mappings
- Zero-decode passthrough when OSC processing is not requested
- OSC message and bundle validation
- Simple address filters and address remapping
- Built-in VMC protocol profile
- Recoverable high-frequency binary recording
- Timed replay and recording inspection
- Windows, macOS, and Linux on x86_64 and ARM64

## Documentation

- [Roadmap](ROADMAP.md) — delivery order, capability status, and acceptance gates
- [Documentation index](docs/README.md) — sources of truth and maintenance rules
- [Architecture](docs/architecture.md) — boundaries, dependency direction, and planned repository layout
- [Configuration specification](docs/specs/configuration.md) — versioned endpoint and mapping schema
- [VMC passthrough example](examples/vmc-passthrough.toml) — receive and forward a VMC OSC stream
- [VMC profile](docs/profiles/vmc.md) — scope of built-in VMC support

## Status

Planning. No Rust implementation has been started.
