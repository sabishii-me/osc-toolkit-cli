# Binary recording format

Status: Draft
Last reviewed: 2026-08-20
Related capabilities: `SPEC-03`, `REC-01`, `REC-04`, `REPLAY-01`

## Scope

Format version 1 stores timestamped raw UDP datagrams. It is payload-agnostic and does not serialize decoded OSC or VMC objects. The initial implementation records the input tap. Mapping and output taps will use the same record structure in a later compatible extension.

All integers are little-endian. Writers reject values that cannot fit their declared widths.

## File header

| Offset | Size | Value |
|---:|---:|---|
| 0 | 8 | Magic: `OSCTKREC` |
| 8 | 2 | Format version: `1` |
| 10 | 2 | Header size: `32` |
| 12 | 4 | Reserved, zero |
| 16 | 8 | Recording start as Unix epoch nanoseconds, unsigned |
| 24 | 8 | Reserved, zero |

The Unix timestamp is a session anchor for alignment with external media. Replay timing does not use wall-clock time.

## Datagram chunks

Each received datagram is one independently recoverable chunk:

| Field | Size | Meaning |
|---|---:|---|
| Magic | 4 | `DGRM` |
| Body length | 4 | Bytes after the chunk prefix |
| CRC32C | 4 | CRC32C of the complete body |
| Monotonic offset | 8 | Nanoseconds since recorder creation |
| Stream ID length | 2 | UTF-8 byte length |
| Flags | 2 | Zero in version 1 |
| Payload length | 4 | Raw datagram byte length |
| Stream ID | variable | Input endpoint identifier |
| Payload | variable | Byte-identical UDP payload |

The body length equals `16 + stream_id_length + payload_length`. Version 1 limits stream IDs to 65,535 bytes, payloads to 65,507 bytes, and chunk bodies to 16 MiB.

The timestamp is sampled immediately after UDP receipt and before forwarding. It comes from a process-local monotonic clock. Records must be emitted in nondecreasing timestamp order. Equal timestamps retain file order.

## Finalization and recovery

Version 1 has no required footer or index. A clean stop flushes file-system buffers and synchronizes file contents. Readers scan sequentially from the header and accept every complete chunk with valid lengths and CRC32C.

An incomplete final prefix, body, or CRC mismatch is a damaged tail. Recovery returns all preceding valid records and reports the byte offset of the damaged tail. Corruption before the final chunk is an error and must not be silently skipped.

## Replay

Timed replay uses monotonic offsets relative to the first selected record. Wall-clock time is never used for packet scheduling. Records with equal offsets keep file order. The replay cursor (`osc_recording::ReplayCursor`) implements these rules for every replay path.

A stream identifier is the input it was recorded from, so streams stay separate on replay:

- **Mapping restoration** (`replay --config`): each stream is sent through the mapping edges of the input with the same identifier, using the same topology compilation as live routing. Inputs are not bound. A stream with no matching input or no edge is skipped and reported.
- **Stream selection** (`replay --stream`, repeatable): only the listed streams are replayed. Naming a stream the file does not contain is an error.
- **Explicit targets** (`replay --target`, repeatable): every selected datagram goes to every target. Sending more than one stream this way merges them, so it is refused unless `--merge-streams` is given.

Range and speed are not implemented yet.

## Compatibility

Released format versions are immutable. Readers reject unknown versions. Additions that alter header or chunk interpretation require a new format version and fixtures for every supported version.
