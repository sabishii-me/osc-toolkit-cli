# osc-toolkit-cli

A cross-platform Rust CLI for high-throughput, bounded-memory UDP/OSC routing, filtering, remapping, recording, and replay.

The project is a **data-plane tool**. Task orchestration, process lifecycle management, configuration deployment, and multi-instance coordination belong to an external orchestration engine.

## Capabilities

Available in 0.2.0 (Windows x86_64 builds on the
[releases page](https://github.com/sabishii-me/osc-toolkit-cli/releases)):

- `validate`: check a configuration file without opening sockets.
- `serve`: N:N UDP passthrough with a local TCP control endpoint for recording.
- `record start | stop | status | shutdown`: control recording on a running
  `serve` without restarting its listeners.
- `replay`: replay a recording with its recorded timing, optionally with
  `--loop`. `--config` restores the routing: each recorded stream goes
  through its input's mappings, as in `serve`. `--stream` (repeatable)
  selects streams. `--target` (repeatable) sends the selected streams to
  explicit addresses. It refuses to merge several streams into one target
  unless `--merge-streams` is given.
- Recordings use the crash-recoverable [binary format](docs/specs/recording-format.md)
  (`OSCTKREC`, CRC32C per datagram, no footer required).

```text
osc-toolkit serve --config examples/vmc-passthrough.toml --control 127.0.0.1:7012
osc-toolkit record --control 127.0.0.1:7012 start --file take-001.oscr
osc-toolkit record --control 127.0.0.1:7012 stop
osc-toolkit replay --file take-001.oscr --config examples/vmc-replay-two-ports.toml
osc-toolkit replay --file take-001.oscr --stream vmc-port-39539 --target 127.0.0.1:39539
```

The project is early. Status per capability is tracked in the [Roadmap](ROADMAP.md).

### Planned

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
- [Recording format](docs/specs/recording-format.md) — timestamp and recovery contract
- [VMC passthrough example](examples/vmc-passthrough.toml) — receive and forward a VMC OSC stream
- [VMC profile](docs/profiles/vmc.md) — scope of built-in VMC support

## License

[MIT](LICENSE)
