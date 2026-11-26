use crate::clipboard::ClipboardEntry;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

pub struct Storage {
    file_path: PathBuf,
    _lock: File,
}

impl Storage {
    pub fn open() -> Result<(Self, Vec<ClipboardEntry>), String> {
        let local_data = std::env::var_os("LOCALAPPDATA").ok_or("找不到 LOCALAPPDATA")?;
        let directory = PathBuf::from(local_data).join("SoloClipboard");
        fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
        let lock = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(directory.join("history.lock"))
            .map_err(|error| error.to_string())?;
        lock.try_lock().map_err(|error| match error {
            fs::TryLockError::WouldBlock => "历史记录正由另一个窗口使用".to_string(),
            fs::TryLockError::Error(error) => error.to_string(),
        })?;
        let file_path = directory.join("history.json");
        let history = match Self::read(&file_path) {
            Ok(history) => history,
            Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(error) => return Err(error.to_string()),
        };
        Ok((
            Self {
                file_path,
                _lock: lock,
            },
            history,
        ))
    }

    pub fn read(path: &Path) -> io::Result<Vec<ClipboardEntry>> {
        serde_json::from_reader(BufReader::new(File::open(path)?)).map_err(io::Error::other)
    }

    pub fn write(path: &Path, history: &[ClipboardEntry]) -> io::Result<()> {
        let mut writer = BufWriter::new(File::create(path)?);
        serde_json::to_writer(&mut writer, history).map_err(io::Error::other)?;
        writer.flush()?;
        writer.get_ref().sync_all()
    }

    pub fn save_history(&self, history: &[ClipboardEntry]) -> Result<(), String> {
        let temporary_path = self.file_path.with_extension("tmp");
        let result = Self::write(&temporary_path, history)
            .and_then(|()| fs::rename(&temporary_path, &self.file_path));
        if result.is_err() {
            let _ = fs::remove_file(&temporary_path);
        }
        result.map_err(|error| error.to_string())
    }
}
