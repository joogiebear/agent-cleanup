//! Core logic for Agent Cleanup, kept free of UI code so it can be tested
//! (and later reused by a command line or background mode).

pub mod actions;
pub mod cache;
pub mod config;
pub mod git;
pub mod model;
pub mod scan;
pub mod update;
