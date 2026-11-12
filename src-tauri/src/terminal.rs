use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};

const MAX_LOG_ENTRIES: usize = 500;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminalLogEntry {
    pub id: u64,
    pub timestamp: String,
    pub level: String, // "INFO", "PROXY", "QUOTA", "RESUME", "WARN", "ERROR"
    pub message: String,
}

static LOG_BUFFER: OnceLock<Mutex<VecDeque<TerminalLogEntry>>> = OnceLock::new();
static COUNTER: OnceLock<Mutex<u64>> = OnceLock::new();

fn get_buffer() -> &'static Mutex<VecDeque<TerminalLogEntry>> {
    LOG_BUFFER.get_or_init(|| Mutex::new(VecDeque::with_capacity(MAX_LOG_ENTRIES)))
}

fn next_id() -> u64 {
    let mutex = COUNTER.get_or_init(|| Mutex::new(1));
    if let Ok(mut count) = mutex.lock() {
        let val = *count;
        *count = val.saturating_add(1);
        val
    } else {
        1
    }
}

pub fn log_terminal(level: &str, message: &str) {
    let now = chrono::Local::now().format("%H:%M:%S").to_string();
    let entry = TerminalLogEntry {
        id: next_id(),
        timestamp: now,
        level: level.to_uppercase(),
        message: message.to_string(),
    };

    if let Ok(mut buf) = get_buffer().lock() {
        if buf.len() >= MAX_LOG_ENTRIES {
            buf.pop_front();
        }
        buf.push_back(entry);
    }
}

pub fn get_logs(since_id: Option<u64>) -> Vec<TerminalLogEntry> {
    if let Ok(buf) = get_buffer().lock() {
        match since_id {
            Some(id) => buf.iter().filter(|entry| entry.id > id).cloned().collect(),
            None => buf.iter().cloned().collect(),
        }
    } else {
        Vec::new()
    }
}

pub fn clear_logs() {
    if let Ok(mut buf) = get_buffer().lock() {
        buf.clear();
    }
}
