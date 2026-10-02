//! Fullscreen TUI — a new presentation layer over the untouched engine.
//!
//! ```bash
//! local-ai tui --project MyApp            # command center (needs a terminal)
//! local-ai tui --no-animation --ascii     # calm + portable mode
//! ```
//!
//! Architecture (the UI never drives the engine directly):
//!
//! ```text
//! Model → Agent Runtime (commands::runner) → Event Bus (core::ui_events)
//!                                              ↓
//!                                    TUI Renderer (this module)
//! ```
//!
//! The TUI subscribes to [`crate::core::ui_events`] and renders panels;
//! approvals flow back through oneshot responders. Every `println!` in the
//! driven path reroutes to the bus while the TUI owns the screen, so all
//! existing subcommands behave exactly as before.
pub mod anime;
pub mod app;
pub mod bridge;
pub mod theme;
