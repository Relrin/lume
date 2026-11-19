//! The files of a package the window is shown: pictures and the shaders of cursor
//! effects.
//!
//! A plugin's pictures are drawn by the window only as images (`<img>`, or a CSS mask
//! for one tinted with the theme's colour), from `data:` addresses made here. An SVG
//! drawn as an image runs no script and loads nothing, so a picture can show only
//! itself; the checks below keep what is sent small and of a kind the window expects.

use std::path::Path;

use crate::manifest::check_package_path;
use crate::shader;

/// The most an SVG may hold, and a picture of any other kind, in bytes.
pub const SVG_MAX_BYTES: u64 = 16 * 1024;
pub const RASTER_MAX_BYTES: u64 = 128 * 1024;

/// The most a plugin's pictures may hold together, in bytes.
pub const IMAGES_MAX_BYTES: u64 = 512 * 1024;

/// The kinds of picture a package may have, by the ending of the file's name.
pub const IMAGE_KINDS: [&str; 6] = ["svg", "png", "jpg", "jpeg", "webp", "gif"];

/// A picture read from a package.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    pub mime: &'static str,
    pub bytes: Vec<u8>,
}

impl Image {
    /// The picture as an address the window can draw it from.
    pub fn data_url(&self) -> String {
        format!("data:{};base64,{}", self.mime, base64(&self.bytes))
    }
}

/// The kind of picture a file is, by its name.
fn kind_of(path: &str) -> Option<&'static str> {
    let lower = path.to_ascii_lowercase();
    let ending = lower.rsplit('.').next()?;
    Some(match ending {
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        _ => return None,
    })
}

/// Whether a file is a picture by its name.
pub fn is_image_path(path: &str) -> bool {
    kind_of(path).is_some()
}

/// Whether bytes begin the way a picture of the kind does.
fn looks_like(mime: &str, bytes: &[u8]) -> bool {
    match mime {
        "image/png" => bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
        "image/jpeg" => bytes.starts_with(&[0xff, 0xd8, 0xff]),
        "image/gif" => bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a"),
        "image/webp" => bytes.len() > 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP",
        _ => false,
    }
}

/// Checks the text of an SVG: one drawing, with nothing that runs or names another
/// document.
fn check_svg(bytes: &[u8]) -> Result<(), String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "an SVG is UTF-8 text".to_owned())?;
    let lower = text.to_ascii_lowercase();
    if !lower.contains("<svg") {
        return Err("it is not an SVG drawing".to_owned());
    }
    for refused in ["<script", "<foreignobject", "javascript:", "<!entity"] {
        if lower.contains(refused) {
            return Err(format!("an SVG may not hold `{refused}`"));
        }
    }
    // Anything named by address must be inside the drawing itself.
    for attribute in ["href=\"", "href='"] {
        for (at, _) in lower.match_indices(attribute) {
            let value = &lower[at + attribute.len()..];
            if !value.starts_with('#') {
                return Err("an SVG may name only its own parts, as href=\"#part\"".to_owned());
            }
        }
    }
    if lower.contains("url(")
        && lower.match_indices("url(").any(|(at, _)| {
            let value = lower[at + 4..].trim_start_matches(['"', '\'', ' ']);
            !value.starts_with('#')
        })
    {
        return Err("an SVG may name only its own parts, as url(#part)".to_owned());
    }
    Ok(())
}

/// Reads and checks a picture of a package.
pub fn read_image(root: &Path, path: &str) -> Result<Image, String> {
    check_package_path(path)?;
    let mime = kind_of(path)
        .ok_or_else(|| format!("`{path}` is not a picture: .{}", IMAGE_KINDS.join(", .")))?;
    let file = root.join(path);
    let size = std::fs::metadata(&file)
        .map_err(|_| format!("there is no {path} in the package"))?
        .len();
    let most = if mime == "image/svg+xml" {
        SVG_MAX_BYTES
    } else {
        RASTER_MAX_BYTES
    };
    if size > most {
        return Err(format!(
            "{path} holds {size} bytes; a picture may hold {most}"
        ));
    }
    let bytes = std::fs::read(&file).map_err(|error| format!("{path}: {error}"))?;
    if mime == "image/svg+xml" {
        check_svg(&bytes).map_err(|why| format!("{path}: {why}"))?;
    } else if !looks_like(mime, &bytes) {
        return Err(format!("{path} is not the kind of picture its name says"));
    }
    Ok(Image { mime, bytes })
}

/// Reads and checks the shader of a cursor effect.
pub fn read_shader(root: &Path, path: &str) -> Result<String, String> {
    check_package_path(path)?;
    let file = root.join(path);
    let size = std::fs::metadata(&file)
        .map_err(|_| format!("there is no {path} in the package"))?
        .len();
    if size > shader::SHADER_MAX_BYTES as u64 {
        return Err(format!(
            "{path} holds {size} bytes; a shader may hold {}",
            shader::SHADER_MAX_BYTES
        ));
    }
    let source = std::fs::read_to_string(&file).map_err(|error| format!("{path}: {error}"))?;
    shader::check(&source).map_err(|why| format!("{path}: {why}"))?;
    Ok(source)
}

/// Bytes as base64, as a `data:` address writes them.
pub fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        let sextet = |shift: u32| ALPHABET[((n >> shift) & 63) as usize] as char;
        out.push(sextet(18));
        out.push(sextet(12));
        out.push(if chunk.len() > 1 { sextet(6) } else { '=' });
        out.push(if chunk.len() > 2 { sextet(0) } else { '=' });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::testing::TempDir;

    #[test]
    fn base64_is_as_every_reader_has_it() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64(&[0xff, 0xfe, 0x00]), "//4A");
    }

    #[test]
    fn pictures_are_read_and_checked() {
        let scratch = TempDir::new("assets");
        let root = scratch.path();
        let write = |name: &str, bytes: &[u8]| {
            let file = root.join(name);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, bytes).unwrap();
        };
        write(
            "icons/cpu.svg",
            b"<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 16 16\"><use href=\"#a\"/></svg>",
        );
        write("icons/dot.png", b"\x89PNG\r\n\x1a\nrest");
        write("icons/fake.png", b"<svg/>");
        write("icons/script.svg", b"<svg><script>alert(1)</script></svg>");
        write(
            "icons/far.svg",
            b"<svg><image href=\"https://example.com/x.png\"/></svg>",
        );
        write("icons/big.svg", &vec![b' '; 20 * 1024]);

        let svg = read_image(root, "icons/cpu.svg").unwrap();
        assert_eq!(svg.mime, "image/svg+xml");
        assert!(
            svg.data_url()
                .starts_with("data:image/svg+xml;base64,PHN2Zy")
        );
        assert_eq!(read_image(root, "icons/dot.png").unwrap().mime, "image/png");
        for (path, says) in [
            ("icons/fake.png", "not the kind"),
            ("icons/script.svg", "<script"),
            ("icons/far.svg", "its own parts"),
            ("icons/big.svg", "may hold"),
            ("icons/none.svg", "no icons/none.svg"),
            ("../x.svg", "never `..`"),
            ("icons/x.bmp", "not a picture"),
        ] {
            let found = read_image(root, path).unwrap_err();
            assert!(found.contains(says), "{path}: {found}");
        }
    }
}
