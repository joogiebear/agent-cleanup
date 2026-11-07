//! Core logic for Agent Cleanup, kept free of UI code so it can be tested
//! (and later reused by a command line or background mode).

pub mod actions;
pub mod activity;
pub mod cache;
pub mod config;
pub mod deps;
pub mod downloads;
pub mod git;
pub mod junk;
pub mod library;
pub mod model;
pub mod scan;
pub mod trail;
pub mod update;
