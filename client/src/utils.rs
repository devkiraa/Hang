use sha2::{Digest, Sha256};
use std::io::{self, ErrorKind};
use std::path::Path;

/// Compute SHA256 hash based only on the file name
pub fn compute_file_hash<P: AsRef<Path>>(path: P) -> io::Result<String> {
    let name = path
        .as_ref()
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| io::Error::new(ErrorKind::InvalidInput, "Invalid file name"))?;

    let mut hasher = Sha256::new();
    hasher.update(name.as_bytes());
    Ok(format!("{:x}", hasher.finalize()))
}

/// Compute SHA256 hash of a string (for URLs)
pub fn compute_string_hash(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Format seconds into MM:SS or HH:MM:SS
pub fn format_time(seconds: f64) -> String {
    let total_secs = seconds.max(0.0) as u64;
    let hours = total_secs / 3600;
    let minutes = (total_secs % 3600) / 60;
    let secs = total_secs % 60;

    if hours > 0 {
        format!("{:02}:{:02}:{:02}", hours, minutes, secs)
    } else {
        format!("{:02}:{:02}", minutes, secs)
    }
}
