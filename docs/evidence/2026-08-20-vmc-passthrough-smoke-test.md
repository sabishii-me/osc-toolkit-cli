# VMC passthrough smoke test — 2026-08-20

Status: Passed
Related capabilities: `ROUTE-01`, `ROUTE-02`, `ROUTE-04`, `VMC-01`

## Purpose

Verify the release build against a live VMC face-capture stream rather than only synthetic localhost fixtures.

## Environment

- Operating system: Windows, x86_64
- Build: `cargo build --release --workspace`
- Sender: Daidayo face capture with VMC output enabled
- Runtime input: `0.0.0.0:39539`
- Runtime output: `127.0.0.1:39540`
- Configuration: [`../../examples/vmc-passthrough.toml`](../../examples/vmc-passthrough.toml)
- Processing mode: raw UDP passthrough

## Procedure

1. Validate the configuration with the release binary.
2. Start `osc-toolkit run` with the VMC passthrough example.
3. Configure the sender to transmit VMC UDP traffic to port `39539`.
4. Bind an independent UDP capture process to output port `39540`.
5. Observe forwarded datagrams for 120 seconds.

## Evidence

The output capture observed:

- 4,904 UDP datagrams.
- 2,349,936 payload bytes.
- OSC bundles beginning with `#bundle`.
- Standalone messages with these addresses:
  - `/VMC/Ext/Root/Pos`
  - `/VMC/Ext/OK`
  - `/VMC/Ext/T`
- Datagram sizes including 20, 48, 68, 100, 512, 608, 1,008, 1,040, 1,116, and 1,124 bytes.
- No runtime error output during the observation.

The source address seen at the output was the runtime's ephemeral local output socket, confirming that the capture observed forwarded traffic rather than traffic sent directly to the receiver.

## Result

Passed. The release binary received and forwarded a sustained live VMC OSC stream through the raw UDP path. This smoke test establishes interoperability for passthrough only; it does not claim VMC schema validation or typed VMC inspection.

Byte identity, fan-out, convergence, and transactional bind failure remain covered by automated runtime tests.
