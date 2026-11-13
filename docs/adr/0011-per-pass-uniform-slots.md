# 11. One uniform buffer with a slot per post-processing pass

Date: 2026-11-13 · Status: **accepted** · Depends on: ADR-0003, ADR-0006

## Context

The post chain runs six passes per frame (bright, four blurs, composite) and each pass needs
different constants: a blur needs its direction and step, the composite needs the CRT parameters.
The obvious implementation is one uniform buffer written immediately before each pass.

That is a trap, and we fell into it: `Queue::write_buffer` calls made *before* `Queue::submit` are
not ordered relative to the passes in that submission — the GPU sees one buffer with **the last
value written**. The symptom was memorable: the bloom blur passes each sampled the composite's
uniforms, where `direction = (0, 0)`, so the 13-tap Gaussian collapsed to a weighted copy of its
input. The chain "worked" (a 1-pixel halo from the half-resolution upsample), the tests passed
their first draft, and nobody saw anything obviously wrong.

## Decision

`crates/render/src/renderer.rs` allocates **one** post uniform buffer of
`POST_SLOT_STRIDE (256 bytes) × 6 slots` and binds it with `has_dynamic_offset: true`:

| Slot | Pass | Distinct values |
|------|------|-----------------|
| 0 | bright | threshold, knee, strength, source = scene size |
| 1 | blur round 1, horizontal | direction `(2, 0)`, source = bloom size |
| 2 | blur round 1, vertical | direction `(0, 2)` |
| 3 | blur round 2, horizontal | direction `(6, 0)` — wider glow |
| 4 | blur round 2, vertical | direction `(0, 6)` |
| 5 | composite | CRT parameters, flash, source = scene size |

Each frame writes all six blocks (64 bytes apiece, into their 256-byte aligned slots), then each
pass is drawn with `set_bind_group(0, bind, &[slot_offset])`.

Struct layouts are pinned by compile-time assertions (`size_of::<Instance>() == 64`,
`size_of::<PostUniforms>() % 16 == 0`) and the WGSL mirrors them field for field, using only
`vec2f`/`vec4f` on 16-byte boundaries so the Rust and WGSL layouts cannot drift apart silently.

## Consequences

- (+) One buffer, one bind group layout, six independent value sets — no aliasing hazard, and the
  failure mode of the "write before submit" version becomes impossible by construction.
- (+) Uniform traffic is ~400 bytes per frame, written in six tiny `write_buffer` calls.
- (−) The 256-byte stride is `min_uniform_buffer_offset_alignment`; that wastes ~190 bytes per slot.
  Irrelevant at this size.
- (−) Adding a seventh pass means adding a slot constant; forgetting to do so produces a validation
  error naming the buffer, which is loud enough.
- (−) Dynamic offsets require the bind group to declare `size: Some(size_of::<PostUniforms>())`;
  binding the whole buffer would forbid any offset at all (wgpu says so explicitly, fortunately).

## Alternatives rejected

- **One buffer written before each pass** — broken as described above, and it *looks* right.
- **One buffer per pass (six buffers)** — correct, and what we had briefly; more objects and more
  bind-group churn for no gain over dynamic offsets.
- **Push constants** — limited to 64–128 bytes across backends, and wgpu exposes them only behind
  `Features::PUSH_CONSTANTS`.
- **A storage buffer with per-pass offsets** — works, but storage bindings in the vertex stage and
  uniform bindings in the fragment stage have different alignment rules; two uniform paths is
  simpler to reason about.
