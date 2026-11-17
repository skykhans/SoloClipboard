use clipboard_win::{formats, get_clipboard_string, set_clipboard_string};
use parking_lot::RwLock;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

pub struct ClipboardMonitor {
    history: Arc<RwLock<Vec<ClipboardEntry>>>,
    last_content: Arc<RwLock<String>>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ClipboardEntry {
    pub content: String,
    pub timestamp: chrono::DateTime<chrono::Local>,
    pub id: usize,
}

impl ClipboardMonitor {
    pub fn new() -> Self {
        Self {
            history: Arc::new(RwLock::new(Vec::new())),
            last_content: Arc::new(RwLock::new(String::new())),
        }
    }

    pub fn start_monitoring(&self) {
        let history = Arc::clone(&self.history);
        let last_content = Arc::clone(&self.last_content);

        thread::spawn(move || {
            loop {
                if let Ok(content) = get_clipboard_string() {
                    let last = last_content.read().clone();

                    if !content.is_empty() && content != last {
                        let mut history_lock = history.write();
                        let id = history_lock.len();

                        let entry = ClipboardEntry {
                            content: content.clone(),
                            timestamp: chrono::Local::now(),
                            id,
                        };

                        history_lock.push(entry);
                        *last_content.write() = content;
                    }
                }

                thread::sleep(Duration::from_millis(500));
            }
        });
    }

    pub fn get_history(&self) -> Vec<ClipboardEntry> {
        self.history.read().clone()
    }

    pub fn copy_to_clipboard(&self, content: &str) -> Result<(), String> {
        set_clipboard_string(content).map_err(|e| e.to_string())?;
        *self.last_content.write() = content.to_string();
        Ok(())
    }

    pub fn clear_history(&self) {
        self.history.write().clear();
    }

    pub fn delete_entry(&self, id: usize) {
        let mut history = self.history.write();
        history.retain(|entry| entry.id != id);
    }

    pub fn delete_entries(&self, ids: &[usize]) {
        let mut history = self.history.write();
        history.retain(|entry| !ids.contains(&entry.id));
    }

    pub fn delete_all_before(&self, timestamp: chrono::DateTime<chrono::Local>) {
        let mut history = self.history.write();
        history.retain(|entry| entry.timestamp > timestamp);
    }
}
