//! Local runtime discovery.
//!
//! The user should not have to remember that LM Studio listens on `1234` while
//! Ollama listens on `11434`. This module sweeps the loopback interface for the
//! ports local inference runtimes are known to bind, fingerprints whatever
//! answers, and reports the model list it exposes.
//!
//! Two signals are collected, and they answer different questions:
//!
//! - **Reachability** — a TCP connect to a candidate port. This is the only
//!   signal that says "a server is running *right now*". One refused connect is
//!   a single syscall on loopback, so the whole sweep is a few microseconds per
//!   closed port and the probes run concurrently.
//! - **Installed** — an app bundle, a well-known install directory, or an
//!   executable on `PATH`. This says "you have the software, start the server".
//!   Without it the UI can only ever show an empty list, which is exactly the
//!   dead end the Docker edition's `ollama_tags` discovery was built to avoid.
//!
//! Ports are shared between runtimes (LocalAI and llama.cpp both default to
//! `8080`, so this sweep probes LocalAI on `8090`), so the port alone cannot
//! identify a server. Every open port is therefore fingerprinted: a handful of cheap
//! vendor-specific endpoints are queried in parallel and the first positive
//! match wins. Ports with no positive match fall back to the runtime the port
//! most commonly belongs to.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use futures_util::future::join_all;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};

use crate::preset::ProviderKind;

/// A closed loopback port is answered by the kernel immediately, so the connect
/// budget only has to cover a loaded machine. The request budget is larger
/// because a loaded model server can take a while to answer a model list.
const CONNECT_TIMEOUT: Duration = Duration::from_millis(400);
const REQUEST_TIMEOUT: Duration = Duration::from_millis(2500);
const TCP_CONNECT_TIMEOUT: Duration = Duration::from_millis(300);

/// Loopback hosts to sweep. `127.0.0.1` covers the overwhelming majority of
/// runtimes, but a server bound only to the IPv6 wildcard (`llama-server
/// --host ::`, some container bridges) is invisible there.
const LOOPBACK_HOSTS: &[&str] = &["127.0.0.1", "[::1]"];

mod enrich;
mod install;
mod models;
mod runtimes;
mod scan;
mod urls;

pub use enrich::*;
pub use install::*;
pub use models::*;
pub use runtimes::*;
pub use scan::*;
pub use urls::*;
