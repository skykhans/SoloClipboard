use crate::clipboard::ClipboardEntry;
use std::fs;
use std::path::PathBuf;

pub struct Storage {
    file_path: PathBuf,
}

impl Storage {
    pub fn new() -> Self {
        let mut path = dirs::data_local_dir().unwrap_or_else(|| PathBuf::from("."));
        path.push("SoloClipboard");

        if !path.exists() {
            let _ = fs::create_dir_all(&path);
        }

        path.push("history.json");

        Self { file_path: path }
    }

    pub fn save_history(&self, history: &[ClipboardEntry]) -> Result<(), String> {
        let json = serde_json::to_string_pretty(history)
            .map_err(|e| format!("序列化失败: {}", e))?;

        fs::write(&self.file_path, json)
            .map_err(|e| format!("写入文件失败: {}", e))?;

        Ok(())
    }

    pub fn load_history(&self) -> Result<Vec<ClipboardEntry>, String> {
        if !self.file_path.exists() {
            return Ok(Vec::new());
        }

        let content = fs::read_to_string(&self.file_path)
            .map_err(|e| format!("读取文件失败: {}", e))?;

        let history: Vec<ClipboardEntry> = serde_json::from_str(&content)
            .map_err(|e| format!("解析失败: {}", e))?;

        Ok(history)
    }
}
