//! HDSP (direct streaming) has no long-running task: each request maps to a
//! single immediate `MoveTo` and is handled inline by the dispatcher
//! (`Dispatcher::hdsp_move` / `hdsp_move_duration`, SPEC §7.4). This module
//! exists for symmetry and documentation.
