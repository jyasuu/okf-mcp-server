use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

use chrono::Utc;
use serde::Serialize;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize)]
pub struct AuditEntry {
    pub id: String,
    pub timestamp: String,
    pub tool: String,
    pub bundle: String,
    pub target_path: String,
    pub caller: String,
    pub params_summary: String,
    pub result: String,
}

pub struct AuditLog {
    file: PathBuf,
    mutex: Mutex<()>,
}

impl AuditLog {
    pub fn new(data_dir: &str) -> Result<Self, std::io::Error> {
        fs::create_dir_all(data_dir)?;
        let file = PathBuf::from(data_dir).join("audit.jsonl");
        Ok(Self {
            file,
            mutex: Mutex::new(()),
        })
    }

    pub fn record(
        &self,
        tool: &str,
        bundle: &str,
        target_path: &str,
        params_summary: &str,
        result: &str,
    ) -> Result<(), std::io::Error> {
        let _guard = self.mutex.lock().unwrap();

        let entry = AuditEntry {
            id: Uuid::new_v4().to_string(),
            timestamp: Utc::now().to_rfc3339(),
            tool: tool.to_string(),
            bundle: bundle.to_string(),
            target_path: target_path.to_string(),
            caller: "unknown".to_string(),
            params_summary: params_summary.to_string(),
            result: result.to_string(),
        };

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.file)?;

        let line = serde_json::to_string(&entry)?;
        writeln!(file, "{line}")?;

        Ok(())
    }

    pub fn record_ok(
        &self,
        tool: &str,
        bundle: &str,
        target_path: &str,
        params_summary: &str,
    ) -> Result<(), std::io::Error> {
        self.record(tool, bundle, target_path, params_summary, "ok")
    }

    pub fn record_error(
        &self,
        tool: &str,
        bundle: &str,
        target_path: &str,
        params_summary: &str,
        error: &str,
    ) -> Result<(), std::io::Error> {
        self.record(
            tool,
            bundle,
            target_path,
            params_summary,
            &format!("error: {error}"),
        )
    }
}
