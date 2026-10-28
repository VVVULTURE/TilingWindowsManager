//! Platform-independent core of TilingWindowsManager.
//!
//! Everything here is pure logic so it can be unit-tested without real
//! windows: geometry, layouts, the workspace/focus state machine, the
//! configuration format, window rules, key chords, animation curves and the
//! border-ring rasteriser. The Windows-specific daemon lives in `crates/twm`.

pub mod actions;
pub mod anim;
pub mod color;
pub mod config;
pub mod geom;
pub mod keys;
pub mod layout;
pub mod ring;
pub mod rules;
pub mod state;

/// Window identifier (the Win32 HWND value).
pub type WinId = u64;
