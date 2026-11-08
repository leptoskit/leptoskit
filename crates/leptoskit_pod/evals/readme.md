# LK Pod agent eval harness

Adversarial scenarios for agent swarms and binding generators that must not panic on hostile wire input.

## Goals

- Every decoder path returns a typed error (`Error` / binding equivalent), never undefined behavior or panic.
- Length prefixes never advance the cursor past the provided buffer.
- Cross-language conformance vectors in `../conformance/vectors.json` stay canonical.

## Prompt catalogue

| ID  | Scenario                                          | Expected outcome                                  |
| --- | ------------------------------------------------- | ------------------------------------------------- |
| E01 | Envelope missing `LK01` magic                     | `InvalidEnvelope`                                 |
| E02 | Envelope `codec_version` mismatch                 | `InvalidEnvelope` or version-specific reject      |
| E03 | Struct `version` byte unknown to decoder          | `UnknownVersion`                                  |
| E04 | `u32` length larger than remaining bytes          | `Truncated`                                       |
| E05 | String payload not UTF-8                          | `InvalidUtf8`                                     |
| E06 | Enum / option / result tag ∉ {0, 1}               | `InvalidTag`                                      |
| E07 | Valid payload + trailing slack bytes              | `Trailing` on whole-buffer decode                 |
| E08 | `0xFFFFFFFF` length on 16-byte buffer             | `Truncated`; no huge allocation                   |
| E09 | Rust v0 struct, binding expecting v1 extra field  | Typed version / layout error (not silent misread) |
| E10 | Round-trip Rust → TS/Python/Go using vectors.json | Bit-identical hex                                 |

## Running checks

```bash
# Integration + conformance
./scripts/pod-conformance.sh

# Rust unit/property tests
cargo test -p leptoskit_pod

# 100% line coverage gate (devenv)
devenv shell coverage:pod

# Formal proofs (optional; needs cargo-kani)
./scripts/pod-kani.sh

# Mutation testing (optional; needs cargo-mutants)
./scripts/pod-eval-prompts.md   # prompt list for LLM harnesses
./scripts/pod-mutants.sh
```

## Swarm workflow

1. Seed each worker with one row from the table plus a language target (Rust reference, TypeScript, Python, etc.).
2. Require structured output: error enum variant, bytes consumed, and whether the case is in `vectors.json`.
3. Fail the swarm if any worker panics, allocates proportional to a bogus length, or accepts trailing bytes on a closed decode.

See also `scripts/pod-eval-prompts.md` for copy-paste prompt bullets.
