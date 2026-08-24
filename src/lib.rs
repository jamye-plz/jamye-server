//! Jamye server library.
//!
//! The crate is a modular monolith. Binaries and HTTP composition live at the
//! outside; application and domain modules remain independent of transports
//! and infrastructure clients.

#![forbid(unsafe_code)]

pub mod adapters;
pub mod application;
pub mod config;
pub mod domain;
pub mod platform;
pub mod ports;
pub mod transport;
