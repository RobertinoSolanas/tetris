//! `tetris-app` — the shell context plus everything that glues the other contexts together.
//!
//! ```text
//!   winit events ─► input::translate ─► Command ─► Session ─► GameEvent
//!                                                                    │
//!                              ViewModel ◄───────────────────────────┤
//!                                 │                                  ├─► audio cues
//!                                 ▼                                  │
//!                           scene_builder::build ─► Scene ─► tetris-render
//!```

pub mod app;
pub mod cli;
pub mod input;
pub mod json;
pub mod layout;
pub mod persistence;
pub mod rect;
pub mod scene_builder;
pub mod theme;
pub mod view_model;

/// Columns in the playfield (mirrors `tetris_core::BOARD_COLS`, used all over the layout code).
pub const FIELD_COLS: usize = 10;
/// Rows in the playfield, buffer included.
pub const FIELD_ROWS: usize = 22;

pub use cli::Options;
pub use layout::Layout;
pub use persistence::{Settings, Store};
pub use rect::Rect;
pub use scene_builder::Screen;
pub use view_model::ViewModel;
