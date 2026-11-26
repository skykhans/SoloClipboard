use crate::desktop::window_hwnd;
use base64::{engine::general_purpose::STANDARD, Engine};
use clipboard_win::{formats::CF_DIB, raw, Clipboard};
use gpui::{App, ClipboardEntry as SystemEntry, Image, ImageFormat, Window};
use std::cell::OnceCell;
use std::collections::HashSet;
use std::io::Cursor;
use std::num::NonZeroU32;
use std::sync::Arc;
use windows_sys::Win32::Foundation::HWND;

#[derive(serde::Serialize, serde::Deserialize)]
pub struct ClipboardEntry {
    pub content: String,
    pub timestamp: chrono::DateTime<chrono::Local>,
    pub id: usize,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<ClipboardImage>,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct ClipboardImage {
    mime: String,
    data: String,
    #[serde(skip)]
    image: OnceCell<Option<Arc<Image>>>,
}

impl ClipboardImage {
    pub fn load(&self) -> Option<Arc<Image>> {
        self.image
            .get_or_init(|| {
                let format = ImageFormat::from_mime_type(&self.mime)?;
                let bytes = STANDARD.decode(&self.data).ok()?;
                Some(Arc::new(Image::from_bytes(format, bytes)))
            })
            .clone()
    }
}

pub struct ClipboardMonitor {
    pub history: Vec<ClipboardEntry>,
    sequence: Option<NonZeroU32>,
    next_id: usize,
}

impl ClipboardMonitor {
    pub fn new(mut history: Vec<ClipboardEntry>) -> Self {
        // Older files can contain duplicate IDs after deletions.
        for (id, entry) in history.iter_mut().enumerate() {
            entry.id = id;
        }
        Self {
            next_id: history.len(),
            sequence: None,
            history,
        }
    }

    pub fn capture(&mut self, cx: &App) -> bool {
        let sequence = clipboard_win::seq_num();
        if sequence.is_none() || sequence == self.sequence {
            return false;
        }
        let Some(item) = cx.read_from_clipboard() else {
            return false;
        };
        self.sequence = sequence;
        let content = item.text().unwrap_or_default();
        let image = item.into_entries().find_map(|entry| match entry {
            SystemEntry::Image(image) => Some(image),
            _ => None,
        });
        if content.is_empty() && image.is_none() {
            return false;
        }
        if self.history.last().is_some_and(|last| {
            last.content.replace("\r\n", "\n") == content
                && last
                    .image
                    .as_ref()
                    .and_then(ClipboardImage::load)
                    .as_deref()
                    == image.as_ref()
        }) {
            return false;
        }
        self.history.push(ClipboardEntry {
            content,
            timestamp: chrono::Local::now(),
            id: self.next_id,
            pinned: false,
            image: image.map(|image| ClipboardImage {
                mime: image.format.mime_type().to_owned(),
                data: STANDARD.encode(&image.bytes),
                image: OnceCell::from(Some(Arc::new(image))),
            }),
        });
        self.next_id += 1;
        true
    }

    pub fn copy_entry(&mut self, id: usize, window: &Window) -> Result<(), String> {
        let entry = self
            .history
            .iter()
            .find(|entry| entry.id == id)
            .ok_or("记录已删除")?;
        let hwnd =
            window_hwnd(window).map_err(|error| format!("无法获取 Windows 窗口句柄：{error}"))?;
        if let Some(image) = &entry.image {
            let image = image.load().ok_or("图片数据无法读取")?;
            write_image(&image, hwnd)?;
        } else {
            let _clipboard = Clipboard::new_for(hwnd).map_err(|error| error.to_string())?;
            raw::empty().map_err(|error| error.to_string())?;
            raw::set_string_with(&entry.content, clipboard_win::options::NoClear)
                .map_err(|error| error.to_string())?;
        }
        self.sequence = clipboard_win::seq_num();
        Ok(())
    }

    pub fn import(&mut self, incoming: Vec<ClipboardEntry>) -> usize {
        // IDs are local to each history file. Deduplicate by time and content.
        let accepted: Vec<_> = {
            let mut known: HashSet<_> = self.history.iter().map(ClipboardEntry::key).collect();
            incoming
                .iter()
                .map(|entry| known.insert(entry.key()))
                .collect()
        };
        let previous_count = self.history.len();
        for (mut entry, accepted) in incoming.into_iter().zip(accepted) {
            if accepted {
                entry.id = self.next_id;
                self.next_id += 1;
                self.history.push(entry);
            }
        }
        self.history.sort_by_key(|entry| entry.timestamp);
        self.history.len() - previous_count
    }
}

fn write_image(image: &Image, hwnd: HWND) -> Result<(), String> {
    let clipboard_format = |name| {
        raw::register_format(name)
            .map(NonZeroU32::get)
            .ok_or_else(|| format!("无法注册剪贴板格式 {name}"))
    };
    let native_format = match image.format {
        ImageFormat::Svg => Some(clipboard_format("image/svg+xml")?),
        ImageFormat::Gif => Some(clipboard_format("GIF")?),
        ImageFormat::Jpeg => Some(clipboard_format("JFIF")?),
        _ => None,
    };

    let mut png = Vec::new();
    let mut bmp = Vec::new();
    let png_format = if image.format == ImageFormat::Svg {
        None
    } else {
        let raster = image::load_from_memory(image.bytes()).map_err(|error| error.to_string())?;
        if image.format != ImageFormat::Png {
            raster
                .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
                .map_err(|error| error.to_string())?;
        }
        let rgb = raster.into_rgb8();
        image::codecs::bmp::BmpEncoder::new(&mut bmp)
            .encode(
                &rgb,
                rgb.width(),
                rgb.height(),
                image::ExtendedColorType::Rgb8,
            )
            .map_err(|error| error.to_string())?;
        Some(clipboard_format("PNG")?)
    };

    let _clipboard = Clipboard::new_for(hwnd).map_err(|error| error.to_string())?;
    raw::empty().map_err(|error| error.to_string())?;
    if let Some(format) = native_format {
        raw::set_without_clear(format, image.bytes()).map_err(|error| error.to_string())?;
    }
    if let Some(format) = png_format {
        let png = if image.format == ImageFormat::Png {
            image.bytes()
        } else {
            &png
        };
        raw::set_without_clear(format, png).map_err(|error| error.to_string())?;
        // CF_DIB uses the BMP data without the 14-byte file header.
        raw::set_without_clear(CF_DIB, &bmp[14..]).map_err(|error| error.to_string())?;
    }
    Ok(())
}

impl ClipboardEntry {
    fn key(&self) -> (chrono::DateTime<chrono::Local>, &str, Option<(&str, &str)>) {
        (
            self.timestamp,
            &self.content,
            self.image
                .as_ref()
                .map(|image| (image.mime.as_str(), image.data.as_str())),
        )
    }
}
