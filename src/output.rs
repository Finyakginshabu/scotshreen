//! Clipboard and PNG output.

use std::path::PathBuf;

/// Puts a `w` x `h` RGBA image on the clipboard.
pub fn copy(w: u32, h: u32, rgba: &[u8]) -> Result<(), String> {
    let mut clipboard = arboard::Clipboard::new().map_err(|e| e.to_string())?;
    clipboard
        .set_image(arboard::ImageData {
            width: w as usize,
            height: h as usize,
            bytes: std::borrow::Cow::Borrowed(rgba),
        })
        .map_err(|e| e.to_string())
}

/// Writes `Pictures/Scotshreen/scotshreen-YYYY-MM-DD-HHMMSS.png` and returns its path. Two
/// saves in the same second get `-2`, `-3` and so on: an existing file is never overwritten.
/// Safe to run on a background thread.
pub fn save(w: u32, h: u32, rgba: &[u8]) -> Result<PathBuf, String> {
    let dir = save_dir().ok_or("no Pictures folder")?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let stem = format!("scotshreen-{}", chrono::Local::now().format("%Y-%m-%d-%H%M%S"));
    for n in 1..1000 {
        let name = if n == 1 { format!("{stem}.png") } else { format!("{stem}-{n}.png") };
        let path = dir.join(name);
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => {
                return match write_png(file, w, h, rgba) {
                    Ok(()) => Ok(path),
                    Err(e) => {
                        // Don't leave a broken, half-written PNG behind (disk full, ...).
                        let _ = std::fs::remove_file(&path);
                        Err(e)
                    }
                };
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.to_string()),
        }
    }
    Err("too many files with the same name".into())
}

/// Where Save puts its files: `Pictures/Scotshreen`.
pub fn save_dir() -> Option<PathBuf> {
    Some(dirs::picture_dir()?.join("Scotshreen"))
}

/// The last two parts of a folder, for messages: "Pictures/Scotshreen".
pub fn short_dir(path: &std::path::Path) -> String {
    let dir = path.parent().unwrap_or(path);
    let parts: Vec<_> = dir.iter().rev().take(2).map(|p| p.to_string_lossy().into_owned()).collect();
    parts.into_iter().rev().collect::<Vec<_>>().join("/")
}

/// Writes a `w` x `h` RGBA image as a PNG at `path`.
pub fn save_to(path: impl AsRef<std::path::Path>, w: u32, h: u32, rgba: &[u8]) -> Result<(), String> {
    write_png(std::fs::File::create(path).map_err(|e| e.to_string())?, w, h, rgba)
}

fn write_png(file: std::fs::File, w: u32, h: u32, rgba: &[u8]) -> Result<(), String> {
    let file = std::io::BufWriter::new(file);
    let mut enc = png::Encoder::new(file, w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.set_compression(png::Compression::Fast);
    let mut writer = enc.write_header().map_err(|e| e.to_string())?;
    writer.write_image_data(rgba).map_err(|e| e.to_string())?;
    writer.finish().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_dir_keeps_the_last_two_folders() {
        let p = std::path::Path::new("/Users/me/Pictures/Scotshreen/a.png");
        assert_eq!(short_dir(p), "Pictures/Scotshreen");
    }
}
