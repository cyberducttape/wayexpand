//! Privacy-preserving structured execution auditing for the Action Broker.

use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::{self, Write},
    os::unix::fs::PermissionsExt,
    path::Path,
    sync::Mutex,
};

const MAX_AUDIT_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct CallerIdentity {
    pub pid: Option<u32>,
    pub executable: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AuditEvent<'a> {
    pub timestamp: u64,
    pub request_id: &'a str,
    pub action_id: &'a str,
    pub caller_pid: Option<u32>,
    pub caller_executable: Option<&'a str>,
    pub policy_hash: &'a str,
    pub start: u64,
    pub finish: u64,
    pub duration_ms: u64,
    pub exit_status: Option<i32>,
    pub timed_out: bool,
    pub output_size: usize,
}

pub struct AuditLogger {
    file: Mutex<File>,
    policy_hash: String,
}

impl AuditLogger {
    pub fn new(path: &Path, policy_hash: String) -> io::Result<Self> {
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        Ok(Self {
            file: Mutex::new(file),
            policy_hash,
        })
    }

    pub fn policy_hash(&self) -> &str {
        &self.policy_hash
    }

    pub fn record(&self, event: &AuditEvent<'_>) -> io::Result<()> {
        let mut line = serde_json::to_vec(event)
            .map_err(|error| io::Error::other(format!("serialize audit event: {error}")))?;
        line.push(b'\n');
        let mut file = self
            .file
            .lock()
            .map_err(|_| io::Error::other("audit log mutex poisoned"))?;
        let current_size = file.metadata()?.len();
        if current_size.saturating_add(line.len() as u64) > MAX_AUDIT_BYTES {
            return Err(io::Error::other("audit log size limit reached"));
        }
        file.write_all(&line)?;
        file.sync_data()
    }
}

pub fn policy_hash(content: &[u8]) -> String {
    let digest = Sha256::digest(content);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_hash_is_stable_and_sha256_shaped() {
        let hash = policy_hash(b"policy");
        assert_eq!(hash.len(), 64);
        assert_eq!(hash, policy_hash(b"policy"));
        assert_ne!(hash, policy_hash(b"other policy"));
    }

    #[test]
    fn audit_event_is_jsonl_without_unstructured_payloads() {
        let path =
            std::env::temp_dir().join(format!("wayexpand-audit-test-{}.jsonl", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let logger = AuditLogger::new(&path, "policy-hash".to_string()).unwrap();
        logger
            .record(&AuditEvent {
                timestamp: 1,
                request_id: "request-1",
                action_id: "cluster-status",
                caller_pid: Some(42),
                caller_executable: Some("/usr/bin/wayexpand"),
                policy_hash: logger.policy_hash(),
                start: 1,
                finish: 2,
                duration_ms: 1,
                exit_status: Some(0),
                timed_out: false,
                output_size: 12,
            })
            .unwrap();
        let line = std::fs::read_to_string(&path).unwrap();
        let value: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(value["action_id"], "cluster-status");
        assert!(value.get("stdout").is_none());
        assert!(value.get("arguments").is_none());
        let _ = std::fs::remove_file(path);
    }
}
