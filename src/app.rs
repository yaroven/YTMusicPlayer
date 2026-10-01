//! Application root: owns the channels between UI, player and sync tasks,
//! and runs the main `tokio::select!` loop (input events, player events,
//! sync results, render tick).
