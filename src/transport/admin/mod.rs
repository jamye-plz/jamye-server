//! Operator command-line transport: the `admin` binary's argument parsing, composition and
//! command execution. It connects to PostgreSQL only and opens no listener.

pub mod cli;
pub mod render;
pub mod runtime;
