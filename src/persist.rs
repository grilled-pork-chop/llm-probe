//! Serialise and deserialise a `GrowResult` for `--output` / `--replay`.

use crate::error::ProbeError;
use crate::metrics::GrowResult;
use std::path::Path;

pub fn save(path: &str, result: &GrowResult) -> Result<(), ProbeError> {
    let json = serde_json::to_string_pretty(result).map_err(ProbeError::Decode)?;
    std::fs::write(Path::new(path), json)
        .map_err(|e| ProbeError::Stream(format!("failed to write {path}: {e}")))?;
    Ok(())
}

pub fn load(path: &str) -> Result<GrowResult, ProbeError> {
    let bytes = std::fs::read(Path::new(path))
        .map_err(|e| ProbeError::Stream(format!("failed to read {path}: {e}")))?;
    serde_json::from_slice(&bytes).map_err(ProbeError::Decode)
}
