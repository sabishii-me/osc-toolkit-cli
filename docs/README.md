# Documentation map

This directory separates normative specifications from delivery planning so the roadmap does not become a second architecture document.

## Sources of truth

| Subject | Source of truth | Purpose |
|---|---|---|
| Product scope and delivery status | [`../ROADMAP.md`](../ROADMAP.md) | Capability IDs, phase gates, and current delivery state |
| Component boundaries and dependencies | [`architecture.md`](architecture.md) | Stable architectural constraints and repository layout |
| VMC support boundary | [`profiles/vmc.md`](profiles/vmc.md) | Normative scope for the built-in VMC profile |
| Configuration schema | [`specs/configuration.md`](specs/configuration.md) | Normative user-facing configuration semantics |
| Recording format | `specs/recording-format.md` (Phase 0) | Normative binary format and recovery rules |
| CLI reference | Generated from the binary after Phase 1 | Commands, flags, defaults, and exit codes |
| Actual behavior | Code and automated tests | Final authority when documentation disagrees with implementation |
| Manual verification evidence | [`evidence/`](evidence/) | Reproducible smoke-test environment, procedure, observations, and scope |

Files marked “Phase 0” must be created before the related implementation begins. Empty speculative documents should not be added.

## Anti-corruption rules

1. **One fact, one authoritative document.** Other documents link to it instead of copying detailed schemas or requirement lists.
2. **The roadmap tracks delivery, not implementation detail.** Architecture and format decisions belong in their normative documents.
3. **Capability IDs are stable.** Tests, pull requests, and release notes should reference IDs such as `ROUTE-01` or `VMC-02`.
4. **A capability cannot be marked complete without evidence.** Its roadmap row must link to tests, benchmark results, or release artifacts.
5. **Behavior-changing pull requests update documentation in the same change.** This includes configuration, CLI, binary format, profile behavior, defaults, and platform support.
6. **CLI documentation is generated, not manually duplicated.** Examples may be handwritten; exhaustive flags and defaults may not.
7. **Recording format versions are immutable after release.** Changes require a new version plus compatibility fixtures.
8. **External specifications are pinned.** A profile document records the exact upstream specification/version before conformance claims are made.
9. **Architecture exceptions are explicit.** A change that violates dependency direction requires a decision record under `decisions/`.
10. **Stale status is visible.** Every roadmap review updates its `Last reviewed` field; CI should eventually verify internal links and generated references.

## Document lifecycle

Normative documents should carry these fields near the top:

```text
Status: Draft | Accepted | Superseded
Last reviewed: YYYY-MM-DD
Related capabilities: IDs
```

When superseded, retain the document only if released files/configurations depend on it; otherwise update it in place through normal review. Decision records are append-only after acceptance, except for typo/link corrections.

## Planned documentation layout

```text
docs/
├── README.md
├── architecture.md
├── profiles/
│   └── vmc.md
├── evidence/
│   └── YYYY-MM-DD-test-name.md
├── specs/
│   ├── configuration.md
│   └── recording-format.md    # Phase 0
└── decisions/                 # Created only when a durable decision is needed
```
