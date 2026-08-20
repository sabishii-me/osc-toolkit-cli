# Configuration specification

Status: Draft
Last reviewed: 2026-08-20
Related capabilities: `SPEC-01`, `ROUTE-02`, `ROUTE-03`

## Scope

This document defines configuration schema version 1 for startup-time UDP endpoint topology. Runtime socket behavior, OSC processing, and recording fields will be added here before their implementations.

A configuration is loaded and validated completely before runtime resources are activated. Unknown fields are errors.

## Top-level schema

```toml
version = 1

[[inputs]]
id = "source-a"
bind = "0.0.0.0:9000"
mode = "passthrough"

[[outputs]]
id = "target-a"
target = "127.0.0.1:10000"

[[mappings]]
id = "source-a-to-target-a"
from = "source-a"
to = "target-a"
```

Top-level fields:

| Field | Type | Required | Meaning |
|---|---|---:|---|
| `version` | unsigned integer | yes | Configuration schema version; V1 accepts only `1` |
| `inputs` | array | no | UDP input declarations in source order |
| `outputs` | array | no | UDP output declarations in source order |
| `mappings` | array | no | Explicit directed edges in source order |
| `mapping_groups` | array | no | Directed edge generators in source order |

Empty topology is structurally valid for `validate`, although a runtime command may require at least one active operation.

## Identifiers

All resource identifiers:

- Must be non-empty.
- May contain only ASCII letters, ASCII digits, `.`, `_`, and `-`.
- Are case-sensitive.
- Must be unique within their resource namespace.

Mappings and mapping groups share one namespace because both generate stable edge owners. Inputs and outputs use separate namespaces.

Generated group edge IDs are reserved runtime identifiers and cannot be referenced by V1 configuration fields.

## Inputs

```toml
[[inputs]]
id = "source-a"
bind = "0.0.0.0:9000"
mode = "passthrough"
```

| Field | Type | Required | Default | Meaning |
|---|---|---:|---|---|
| `id` | identifier | yes | — | Stable input identifier |
| `bind` | socket address | yes | — | Numeric IPv4 or bracketed IPv6 address and UDP port |
| `mode` | enum | no | `passthrough` | Payload interpretation mode |

Supported modes:

- `passthrough`: no OSC decoding is required.
- `osc`: the datagram is eligible for OSC validation and processing.

Hostnames are not accepted in V1 input or output socket fields. This keeps validation deterministic and avoids DNS changes during startup.

## Outputs

```toml
[[outputs]]
id = "target-a"
target = "127.0.0.1:10000"
```

| Field | Type | Required | Meaning |
|---|---|---:|---|
| `id` | identifier | yes | Stable output identifier |
| `target` | socket address | yes | Numeric IPv4 or bracketed IPv6 address and UDP port |

## Explicit mappings

```toml
[[mappings]]
id = "source-a-to-target-a"
from = "source-a"
to = "target-a"
```

Each mapping creates one directed input-to-output edge.

| Field | Type | Required | Meaning |
|---|---|---:|---|
| `id` | identifier | yes | Stable edge owner identifier |
| `from` | input identifier | yes | Existing input reference |
| `to` | output identifier | yes | Existing output reference |

## Mapping groups

### All-to-all

```toml
[[mapping_groups]]
id = "stage"
from = ["source-a", "source-b"]
to = ["target-a", "target-b"]
mode = "all-to-all"
```

Expansion order is source-major, then destination-major:

```text
source-a -> target-a   stage[0,0]
source-a -> target-b   stage[0,1]
source-b -> target-a   stage[1,0]
source-b -> target-b   stage[1,1]
```

### Zip

```toml
[[mapping_groups]]
id = "pairs"
from = ["source-a", "source-b"]
to = ["target-b", "target-a"]
mode = "zip"
```

Expansion preserves positional pairing:

```text
source-a -> target-b   pairs[0]
source-b -> target-a   pairs[1]
```

A zip group requires equal non-zero input and output counts. All group references must resolve.

## Combined ordering and duplicate policy

Resolved mappings are ordered as follows:

1. Explicit mappings in declaration order.
2. Mapping groups in declaration order.
3. Edges within each group in the expansion order defined above.

Only one edge may exist for a given `(input, output)` pair. A duplicate produced by any combination of explicit mappings and groups is a validation error. Duplicate edges are rejected rather than silently deduplicated because future edge-level processing and recording settings could otherwise become ambiguous.

## Versioning

- The `version` field is mandatory.
- A binary must reject unsupported versions instead of guessing compatibility.
- Released schema versions are immutable.
- Additive fields that change accepted files require an explicit compatibility review.
- Breaking changes require a new schema version and migration guidance.

## Error behavior

Validation must reject at least:

- Invalid TOML.
- Missing required fields.
- Unknown fields.
- Unsupported schema versions.
- Invalid or duplicate identifiers.
- Unknown input/output references.
- Empty mapping groups.
- Zip length mismatches.
- Duplicate resolved edges.
- Invalid socket-address syntax or out-of-range ports.

Errors must identify the responsible resource and must not activate a partial topology.
