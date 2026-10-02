//! Privacy-preserving structured execution auditing for the Action Broker.

use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::{self, Write},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::{self, SyncSender},
        Arc,
    },
    thread,
    time::Duration,
};

const MAX_AUDIT_BYTES: u64 = 16 * 1024 * 1024;
const AUDIT_QUEUE_CAPACITY: usize = 256;
const AUDIT_BATCH_SIZE: usize = 64;
const AUDIT_BATCH_WAIT: Duration = Duration::from_millis(100);

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
    sender: Option<SyncSender<AuditMessage>>,
    worker: Option<thread::JoinHandle<()>>,
    policy_hash: String,
    dropped_events: Arc<AtomicU64>,
    write_failures: Arc<AtomicU64>,
}

enum AuditMessage {
    Event(Vec<u8>),
    FlushAndStop,
}

impl AuditLogger {
    pub fn new(path: &Path, policy_hash: String) -> io::Result<Self> {
        let writer = AuditWriter::open(path)?;
        let (sender, receiver) = mpsc::sync_channel(AUDIT_QUEUE_CAPACITY);
        let dropped_events = Arc::new(AtomicU64::new(0));
        let write_failures = Arc::new(AtomicU64::new(0));
        let writer_failures = Arc::clone(&write_failures);
        let worker = thread::Builder::new()
            .name("wayexpand-audit".to_string())
            .spawn(move || run_writer(writer, receiver, writer_failures))
            .map_err(|error| io::Error::other(format!("failed to start audit writer: {error}")))?;
        Ok(Self {
            sender: Some(sender),
            worker: Some(worker),
            policy_hash,
            dropped_events,
            write_failures,
        })
    }

    pub fn policy_hash(&self) -> &str {
        &self.policy_hash
    }

    pub fn record(&self, event: &AuditEvent<'_>) -> io::Result<()> {
        let mut line = serde_json::to_vec(event)
            .map_err(|error| io::Error::other(format!("serialize audit event: {error}")))?;
        line.push(b'\n');
        let sender = self
            .sender
            .as_ref()
            .ok_or_else(|| io::Error::other("audit writer is shutting down"))?;
        match sender.try_send(AuditMessage::Event(line)) {
            Ok(()) => Ok(()),
            Err(mpsc::TrySendError::Full(AuditMessage::Event(_))) => {
                self.dropped_events.fetch_add(1, Ordering::Relaxed);
                Err(io::Error::other("audit queue is full; event dropped"))
            }
            Err(mpsc::TrySendError::Full(AuditMessage::FlushAndStop))
            | Err(mpsc::TrySendError::Disconnected(_)) => {
                self.dropped_events.fetch_add(1, Ordering::Relaxed);
                Err(io::Error::other(
                    "audit writer is unavailable; event dropped",
                ))
            }
        }
    }

    pub fn dropped_events(&self) -> u64 {
        self.dropped_events.load(Ordering::Relaxed)
    }

    pub fn write_failures(&self) -> u64 {
        self.write_failures.load(Ordering::Relaxed)
    }

    fn shutdown(&mut self) {
        if let Some(sender) = self.sender.take() {
            let _ = sender.send(AuditMessage::FlushAndStop);
        }
        if let Some(worker) = self.worker.take() {
            if worker.join().is_err() {
                self.write_failures.fetch_add(1, Ordering::Relaxed);
                tracing::error!("audit writer thread panicked during shutdown");
            }
        }
    }
}

impl Drop for AuditLogger {
    fn drop(&mut self) {
        self.shutdown();
    }
}

struct AuditWriter {
    path: PathBuf,
    file: File,
}

impl AuditWriter {
    fn open(path: &Path) -> io::Result<Self> {
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        Ok(Self {
            path: path.to_owned(),
            file,
        })
    }

    fn write_batch(&mut self, batch: &[Vec<u8>]) -> io::Result<()> {
        let bytes = batch.iter().map(Vec::len).sum::<usize>() as u64;
        if bytes > MAX_AUDIT_BYTES {
            return Err(io::Error::other("audit batch exceeds size limit"));
        }
        if self.file.metadata()?.len().saturating_add(bytes) > MAX_AUDIT_BYTES {
            self.rotate()?;
        }
        for line in batch {
            self.file.write_all(line)?;
        }
        self.file.sync_data()
    }

    fn rotate(&mut self) -> io::Result<()> {
        let rotated = self.path.with_file_name(format!(
            "{}.1",
            self.path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("action-audit.jsonl")
        ));
        drop(std::mem::replace(
            &mut self.file,
            OpenOptions::new().write(true).open(&self.path)?,
        ));
        let _ = std::fs::remove_file(&rotated);
        std::fs::rename(&self.path, &rotated)?;
        self.file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        std::fs::set_permissions(&self.path, std::fs::Permissions::from_mode(0o600))?;
        Ok(())
    }
}

fn run_writer(
    mut writer: AuditWriter,
    receiver: mpsc::Receiver<AuditMessage>,
    write_failures: Arc<AtomicU64>,
) {
    while let Ok(message) = receiver.recv() {
        let AuditMessage::Event(first) = message else {
            break;
        };
        let mut batch = vec![first];
        let mut stop_after_batch = false;
        while batch.len() < AUDIT_BATCH_SIZE {
            match receiver.recv_timeout(AUDIT_BATCH_WAIT) {
                Ok(AuditMessage::Event(line)) => batch.push(line),
                Ok(AuditMessage::FlushAndStop) => {
                    stop_after_batch = true;
                    break;
                }
                Err(mpsc::RecvTimeoutError::Timeout | mpsc::RecvTimeoutError::Disconnected) => {
                    break
                }
            }
        }
        if let Err(error) = writer.write_batch(&batch) {
            write_failures.fetch_add(1, Ordering::Relaxed);
            tracing::error!(error = %error, events = batch.len(), "failed to write action audit batch");
            if let Ok(reopened) = AuditWriter::open(&writer.path) {
                writer = reopened;
            }
        }
        if stop_after_batch {
            break;
        }
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
        drop(logger);
        let line = std::fs::read_to_string(&path).expect("audit writer should flush on drop");
        let value: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(value["action_id"], "cluster-status");
        assert!(value.get("stdout").is_none());
        assert!(value.get("arguments").is_none());
        let _ = std::fs::remove_file(path);
    }
}
