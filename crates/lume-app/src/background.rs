//! The picture shown behind the panes.
//!
//! The settings name a file. A page cannot read a file, and is not given the means to
//! read whichever file it likes: it asks for the picture and is handed the bytes of the
//! one file the settings name, if that file is a picture of a kind a webview draws and
//! is not too large to hand over.

use std::io::Read;
use std::path::{Path, PathBuf};

use crate::error::CommandError;

/// The code of the refusal for a picture that is too large.
const TOO_LARGE: &str = "too_large";

/// The kind of picture a file is, told from how it begins. A file's name says nothing
/// that can be relied on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Picture {
    Png,
    Jpeg,
    WebP,
    Gif,
}

impl Picture {
    /// The kind of picture `bytes` begin as, if they begin as one that is shown.
    pub fn of(bytes: &[u8]) -> Option<Self> {
        const PNG: &[u8] = b"\x89PNG\r\n\x1a\n";
        const JPEG: &[u8] = b"\xff\xd8\xff";
        if bytes.starts_with(PNG) {
            Some(Self::Png)
        } else if bytes.starts_with(JPEG) {
            Some(Self::Jpeg)
        } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
            Some(Self::Gif)
        } else if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
            Some(Self::WebP)
        } else {
            None
        }
    }
}

/// The file the settings mean by `written`: a path that is not absolute is counted from
/// the folder the settings are in.
pub fn resolve(config_dir: &Path, written: &str) -> PathBuf {
    let path = Path::new(written);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        config_dir.join(path)
    }
}

/// Reads the picture at `path`, if it is one and is no larger than `max` bytes.
pub fn read(path: &Path, max: u64) -> Result<Vec<u8>, CommandError> {
    let missing = || CommandError::not_found(format!("there is no picture at {}", path.display()));
    let file = std::fs::File::open(path).map_err(|_| missing())?;
    let length = file.metadata().map_err(|_| missing())?.len();
    let too_large = || {
        CommandError::new(
            TOO_LARGE,
            format!(
                "the picture at {} is larger than {} MiB",
                path.display(),
                max / (1024 * 1024)
            ),
        )
    };
    if length > max {
        return Err(too_large());
    }
    // The file may have grown since it was measured: no more than the limit is read.
    let mut bytes = Vec::new();
    file.take(max + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| CommandError::internal(error.to_string()))?;
    if bytes.len() as u64 > max {
        return Err(too_large());
    }
    if Picture::of(&bytes).is_none() {
        return Err(CommandError::unsupported(format!(
            "{} is not a PNG, JPEG, WebP or GIF picture",
            path.display()
        )));
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";

    #[test]
    fn a_picture_is_known_by_how_it_begins() {
        assert_eq!(Picture::of(PNG), Some(Picture::Png));
        assert_eq!(
            Picture::of(b"\xff\xd8\xff\xe0\0\x10JFIF"),
            Some(Picture::Jpeg)
        );
        assert_eq!(Picture::of(b"GIF89a\x01\0\x01\0"), Some(Picture::Gif));
        assert_eq!(Picture::of(b"RIFF\x24\0\0\0WEBPVP8 "), Some(Picture::WebP));
    }

    #[test]
    fn what_is_not_a_picture_is_nothing_whatever_it_is_called() {
        for bytes in [
            &b""[..],
            b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>",
            b"RIFF\x24\0\0\0WAVEfmt ",
            b"RIFF",
            b"\x89PNG",
            b"plain text",
        ] {
            assert_eq!(Picture::of(bytes), None, "{bytes:?}");
        }
    }

    #[test]
    fn a_path_that_is_not_absolute_is_counted_from_the_settings_folder() {
        let dir = TempDir::new("background-resolve");
        let root = dir.path();
        assert_eq!(resolve(root, "wall.png"), root.join("wall.png"));
        assert_eq!(
            resolve(root, "pictures/wall.png"),
            root.join("pictures/wall.png")
        );
        let elsewhere = root.join("elsewhere").join("wall.png");
        assert_eq!(
            resolve(Path::new("other"), &elsewhere.to_string_lossy()),
            elsewhere
        );
    }

    #[test]
    fn a_picture_within_the_limit_is_read_whole() {
        let dir = TempDir::new("background-read");
        let file = dir.path().join("wall.bin");
        std::fs::write(&file, PNG).unwrap();
        assert_eq!(read(&file, PNG.len() as u64).unwrap(), PNG);
    }

    #[test]
    fn a_missing_file_a_large_one_and_one_that_is_no_picture_are_each_refused() {
        let dir = TempDir::new("background-refused");
        let missing = dir.path().join("absent.png");
        assert_eq!(read(&missing, 1024).unwrap_err().code, "not_found");

        let large = dir.path().join("large.png");
        std::fs::write(&large, PNG).unwrap();
        assert_eq!(
            read(&large, PNG.len() as u64 - 1).unwrap_err().code,
            "too_large"
        );

        let text = dir.path().join("text.png");
        std::fs::write(&text, b"not a picture").unwrap();
        assert_eq!(read(&text, 1024).unwrap_err().code, "unsupported");
    }
}
