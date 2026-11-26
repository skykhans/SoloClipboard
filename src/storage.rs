use crate::clipboard::ClipboardEntry;
use crate::desktop::Settings;
use serde::{de::DeserializeOwned, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

pub struct Storage {
    directory: PathBuf,
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
        let history = match Self::read(&directory.join("history.json")) {
            Ok(history) => history,
            Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(error) => return Err(error.to_string()),
        };
        Ok((
            Self {
                directory,
                _lock: lock,
            },
            history,
        ))
    }

    pub fn read(path: &Path) -> io::Result<Vec<ClipboardEntry>> {
        Self::read_json(path)
    }

    pub fn write(path: &Path, history: &[ClipboardEntry]) -> io::Result<()> {
        Self::write_json(path, history)
    }

    pub fn save_history(&self, history: &[ClipboardEntry]) -> Result<(), String> {
        Self::replace_json(&self.directory.join("history.json"), history)
    }

    pub fn load_settings(&self) -> io::Result<Settings> {
        match Self::read_json(&self.directory.join("settings.json")) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Settings::default()),
            result => result,
        }
    }

    pub fn save_settings(&self, settings: Settings) -> Result<(), String> {
        Self::replace_json(&self.directory.join("settings.json"), &settings)
    }

    fn read_json<T: DeserializeOwned>(path: &Path) -> io::Result<T> {
        serde_json::from_reader(BufReader::new(File::open(path)?)).map_err(io::Error::other)
    }

    fn write_json(path: &Path, value: &(impl Serialize + ?Sized)) -> io::Result<()> {
        let mut writer = BufWriter::new(File::create(path)?);
        serde_json::to_writer(&mut writer, value).map_err(io::Error::other)?;
        writer.flush()?;
        writer.get_ref().sync_all()
    }

    fn replace_json(path: &Path, value: &(impl Serialize + ?Sized)) -> Result<(), String> {
        let temporary_path = path.with_extension("tmp");
        let result = Self::write_json(&temporary_path, value)
            .and_then(|()| fs::rename(&temporary_path, path));
        if result.is_err() {
            let _ = fs::remove_file(&temporary_path);
        }
        result.map_err(|error| error.to_string())
    }
}
