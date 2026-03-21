# Generate Module

This directory contains the `mcp generate` implementation along with a Rust
schema-to-TypeScript compiler used for generated tool wrapper types.

The schema normalization and rendering logic in
[`schema_to_ts.rs`](/Users/maxwellgurewitz/.codex/worktrees/4fc9/code-mode/code-mode/src/mcp/generate/schema_to_ts.rs)
was adapted with the TypeScript implementation in
[`bcherny/json-schema-to-typescript`](https://github.com/bcherny/json-schema-to-typescript)
as a reference, specifically revision `43ba08b117adcce82ae2f13e9abd73314680a695`.

The compiler follows the same broad architecture as the upstream project, but
in a smaller Rust-specific form:

1. `Normalizer`
   Takes raw JSON Schema and rewrites it into a more regular shape before code
   generation. This is where we canonicalize `id` to `$id`, `definitions` to
   `$defs`, normalize unary `type` arrays, default `required` and
   `additionalProperties`, and expand bounded arrays into tuple-like forms.

2. `Parser / classifier`
   Walks the normalized schema and decides what TypeScript construct each node
   should become. In this port that logic is lighter-weight than upstream's AST
   model, but it still performs the same conceptual job: distinguish object,
   array, enum, union, intersection, and `$ref` cases and preserve named
   definitions for reuse.

3. `Generator`
   Renders the parsed structure into TypeScript declarations. That includes
   choosing between `interface` and `type`, emitting nested object members,
   tuple unions for bounded arrays, index signatures for
   `additionalProperties`, and named declarations for `$defs` references.

4. `SDK renderer`
   Wraps those generated declarations in the actual SDK files for Code Mode:
   `client.ts`, per-tool files, per-server `index.ts`, the top-level
   `index.ts`, and the manifest.

The Rust tests here were also translated from a subset of that project's
normalizer and end-to-end fixtures so that we preserve the behavior we care
about without depending on the upstream Node toolchain at runtime.
