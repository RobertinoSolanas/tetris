# Ubiquitous Language

Domain terms used consistently across code, tests, docs and UI. If the code says something
different from this glossary, the code is wrong or this file is — pick one and fix it in the
same commit.

## Board geometry

| Term | Code | Meaning |
|---|---|---|
| **Playfield** | `Board` | The 10 × 20 visible well plus a 2-row **buffer** above it where pieces spawn. |
| **Cell** | `Cell { col, row }` | One grid square. `col` grows right, `row` grows **down**; `row = 0` is the topmost buffer row. |
| **Buffer rows** | `Board::BUFFER_ROWS` | Rows hidden behind the HUD; a piece resting entirely in the buffer is not yet a top-out. |
| **Stack** | `Board::occupied` | All locked cells. |
| **Ghost** | `Board::ghost_row` | The lowest legal row for the active piece, drawn as an outline. |
| **Top out** | `GameEvent::GameOver` | A newly spawned piece cannot be placed without overlapping ⇒ the match ends. |

## Pieces

| Term | Code | Meaning |
|---|---|---|
| **Tetromino** | `Tetromino::{I,J,L,O,S,T,Z}` | The seven pieces, identified by letter. |
| **Rotation state** | `Rotation::{Spawn, Cw, Flip, Ccw}` | The four SRS orientations, index 0..3. |
| **Kick** | `kicks::kick_attempts` | The ordered list of offsets tried when a rotation collides (SRS). |
| **Kick used** | `Kick::Offset` vs `Kick::None` | Whether the last successful rotation needed a non-zero offset; feeds T-spin detection. |
| **Active piece** | `ActivePiece` | The falling tetromino: kind + rotation + board position. |
| **Hold** | `Session::hold` | Reserved piece slot, swappable **once per piece**. |
| **Queue / preview** | `PieceSupply::preview` | Upcoming pieces (5 shown), straight from the bag. |
| **Bag** | `SevenBag` | A shuffled set of all seven tetrominoes; refilled when empty. |

## Scoring & progression

| Term | Code | Meaning |
|---|---|---|
| **Clear** | `ClearKind::{None,Single,Double,Triple,Tetris}` | Number of rows cleared by one lock. |
| **T-Spin** | `SpinKind::{None,Mini,Full}` | 3-corner rule on a T lock; `Full` when the front two corners are filled or a kick moved the piece. |
| **Perfect Clear** | `Clear::PerfectClear` | The board is empty after the clear and spawn. |
| **Combo** | `ScoreState::combo` | Consecutive locking pieces that each cleared ≥ 1 row. |
| **Back-to-back (B2B)** | `ScoreState::b2b` | Consecutive "hard clears" (Tetris or any T-Spin clear). |
| **Level** | `Session::level` | Progression step; multiplies score and speeds up gravity. |
| **Goal** | `Progression::goal` | Lines per level (10 by default) required to advance. |
| **Gravity** | `Progression::gravity_ms` | Millimetres per natural drop step for a level (Guideline curve). |
| **Lock delay** | `HandlingConfig::lock_delay` | Time a grounded piece may survive before locking. |
| **Move reset** | `LockTimer::resets_left` | Budget of successful moves/rotations that refresh the lock delay. |
| **DAS / ARR** | `HandlingConfig::{das, arr}` | Delayed Auto Shift / Auto Repeat Rate for held left/right. |
| **SDF** | `HandlingConfig::soft_drop_factor` | Gravity multiplier while soft drop is held. |

## Session lifecycle

| Term | Code | Meaning |
|---|---|---|
| **Session** | `Session` | Aggregate root of one match: owns everything gameplay-related. |
| **Phase** | `Phase::{Title, Playing, Paused, GameOver}` | Where the match is in its lifecycle. |
| **Command** | `Command` | An intent from the shell ("rotate CW"), *before* interpretation by handling rules. |
| **Event** | `GameEvent` | Something that happened in the domain; the only channel to render/sound. |
| **Stats** | `Stats` | Score, lines, pieces, time, tetrises, T-spins, max combo, level. |
| **Seed** | `Session::seed` | The 64-bit value making a run reproducible. |
| **Replay** | `InputLog` | Ordered `(tick, Command)` pairs that reproduce a run exactly. |

## Presentation

| Term | Code | Meaning |
|---|---|---|
| **Scene** | `render::Scene` | Renderer-facing description of one frame: layered quads, text runs, particles, FX amounts. Deliberately contains **no** Tetris concepts. |
| **Scene Builder** | `app::SceneBuilder` | Anti-corruption layer translating domain state + animation state into a `Scene` (rows/cols → pixels, colours → RGBA). |
| **View model** | `app::ViewModel` | Presentation-only animation state (clear animation timeline, particles, shake, trails, toasts) driven by domain events and `tick(dt)`. |
| **Glow** | `Quad::glow` | Per-quad emissive strength feeding the bloom pass. |
| **Post FX** | `PostFx` | Bright-pass, bloom, CRT scanlines, barrel distortion, vignette, chromatic aberration. |
| **Cue** | `audio::Cue` | A short synthesised sound (`Cue::Tetris`, `Cue::Lock`, …). |
