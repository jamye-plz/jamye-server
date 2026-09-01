//! Task-12 C2 generation RED matrix.  This target intentionally checks emitted
//! artifacts rather than scanning source text or creating a second contract test target.

use std::{
    collections::BTreeSet,
    fs, io,
    path::{Component, Path, PathBuf},
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
};

use serde::Deserialize;
use serde_json::Value;

use crate::{TestResult, contract_snapshot};

include!("contract_generation/mapping_oracles.rs");
include!("contract_generation/generation_cases.rs");
include!("contract_generation/generation_assertions.rs");
include!("contract_generation/filesystem_support.rs");
include!("contract_generation/directory_guards.rs");
include!("contract_generation/owned_paths.rs");
