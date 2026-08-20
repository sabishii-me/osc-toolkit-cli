# Built-in VMC profile

Status: Draft
Last reviewed: 2026-08-20
Related capabilities: `VMC-01`, `VMC-02`, `VMC-03`, `VMC-04`

## Decision

VMC support belongs in this repository and binary as an optional, built-in OSC protocol profile. It is not a fork, derivative repository, dynamic plugin, or independent runtime.

VMC is interpreted above the generic OSC codec:

```text
UDP -> OSC codec -> optional VMC profile
```

No VMC work occurs for passthrough traffic or OSC traffic without `profile = "vmc"`.

## Why it belongs here

VMC defines OSC addresses and argument schemas. Supporting those schemas enables validation, inspection, filtering, and finite field remapping without introducing task orchestration or animation behavior. A separate repository would duplicate CLI, transport, recording, release, and compatibility concerns.

A separate motion-processing project may later depend on this toolkit if it needs stateful retargeting or protocol conversion. That does not require removing the VMC specification profile from this repository.

## In scope

### Schema and validation

- Recognize VMC messages defined by the pinned upstream specification.
- Validate OSC argument count and types.
- Reject or report non-finite numeric fields according to an explicit policy.
- Distinguish invalid OSC, valid OSC that violates VMC schema, known-valid VMC, and unknown extension messages.
- Preserve unknown extensions by default.

The exact supported address/schema table must be derived from and tested against a pinned VMC specification version before `VMC-01` is marked complete. This document intentionally does not duplicate that future generated/verified table.

### Typed inspection

- Render known messages using VMC field names.
- Support stable machine-readable output for tests and orchestration tooling.
- Fall back to generic OSC inspection for unknown extension messages.

### Spec-aware filtering

Finite selectors may include:

- VMC message category/type
- Bone name
- Blendshape name
- Tracker/device identifier where the pinned schema defines one

This is schema-aware field selection, not a general expression engine.

### Spec-aware remapping

Exact-name remapping may include:

- Bone names
- Blendshape names
- Tracker/device identifiers where defined by the pinned schema
- Optional explicit case normalization

Remapping modifies only fields identified by the VMC schema. It must not silently modify numeric transforms or unknown messages.

## Out of scope

- Live Link Face binary to VMC conversion
- Frame generation or fixed-rate output
- Latest-frame state retention
- Automatic insertion of missing bones/blendshapes/messages
- Multi-source frame synchronization or merging
- Bone retargeting
- Coordinate-system conversion
- Quaternion/Euler conversion
- Blendshape arithmetic, curves, or composition
- Avatar, character, or application logic
- Process/task orchestration

These are stateful processing or protocol-adapter responsibilities, not VMC specification support.

## Recording behavior

The recorder stores the original or transformed OSC datagram according to tap position. It does not serialize typed VMC objects. Stream metadata may declare `profile = "vmc"` as an interpretation hint, but that hint does not change the binary payload format.

This preserves:

- Unknown extensions
- Exact packet and bundle representation
- Profile-version independence of the recording container
- Direct replay without decoding

## Compatibility policy

Before claiming VMC conformance:

1. Record the authoritative upstream specification URL and revision/version.
2. Create fixtures for every supported message schema.
3. Create invalid fixtures for argument count/type boundaries.
4. Define unknown-extension behavior.
5. State any unsupported or ambiguous parts explicitly.

A later VMC specification revision must not silently change validation of an existing profile version. If revisions are incompatible, configuration must be able to select a profile version.
