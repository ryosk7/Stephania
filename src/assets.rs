//! Image assets for scene v2 image layers (docs/SCENE.md).
//!
//! The control plane stores uploads content-addressed as
//! `<STEPHANIA_ASSETS_DIR>/<id>.png`, where `id` is the first 16 hex digits of
//! the file's SHA-256. The engine loads them only while preparing a Program
//! (never on the media thread), re-checks the hash so a swapped file is never
//! composed, enforces size limits and decodes to straight RGBA8. Decoded
//! images are cached because the same logo appears in many scenes; the cache
//! is bounded by decoded bytes (a 4096x4096 image is 64 MiB of RGBA).

use crate::compositor::Rgba;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

pub const DEFAULT_DIR: &str = "/var/lib/stephania/assets";
pub const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;
pub const MAX_DIMENSION: u32 = 4096;
const CACHE_ENTRIES: usize = 32;
/// Decoded bytes the cache may hold (oldest entries are evicted first).
pub const CACHE_BYTES: usize = 128 * 1024 * 1024;

pub fn valid_id(id: &str) -> bool {
    id.len() == 16 && id.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub struct AssetStore {
    dir: PathBuf,
    cache: Mutex<Vec<(String, Arc<Rgba>)>>,
}

impl AssetStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into(), cache: Mutex::new(Vec::new()) }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn load(&self, id: &str) -> Result<Arc<Rgba>, String> {
        if !valid_id(id) {
            return Err("invalid asset id".into());
        }
        if let Some((_, img)) = self.cache.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|(k, _)| k == id) {
            return Ok(Arc::clone(img));
        }
        let path = self.dir.join(format!("{id}.png"));
        let meta = std::fs::metadata(&path).map_err(|e| format!("asset {id}: {e}"))?;
        if !meta.is_file() || meta.len() > MAX_FILE_BYTES {
            return Err(format!("asset {id}: not a file or larger than {MAX_FILE_BYTES} bytes"));
        }
        let bytes = std::fs::read(&path).map_err(|e| format!("asset {id}: {e}"))?;
        let digest = ring::digest::digest(&ring::digest::SHA256, &bytes);
        let hex: String = digest.as_ref().iter().take(8).map(|b| format!("{b:02x}")).collect();
        if hex != id {
            return Err(format!("asset {id}: content does not match its id"));
        }
        let image = Arc::new(decode_png(&bytes)?);
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        insert_bounded(&mut cache, id, &image);
        Ok(image)
    }
}

/// Decode a PNG of any colour type/bit depth to straight RGBA8.
pub fn decode_png(bytes: &[u8]) -> Result<Rgba, String> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16 | png::Transformations::ALPHA);
    let mut reader = decoder.read_info().map_err(|e| format!("png: {e}"))?;
    let info = reader.info();
    if info.width == 0 || info.height == 0 || info.width > MAX_DIMENSION || info.height > MAX_DIMENSION {
        return Err(format!("png: {}x{} outside 1..={MAX_DIMENSION}", info.width, info.height));
    }
    let size = reader.output_buffer_size().ok_or("png: size overflow")?;
    let mut buf = vec![0; size];
    let frame = reader.next_frame(&mut buf).map_err(|e| format!("png: {e}"))?;
    let (w, h) = (frame.width as usize, frame.height as usize);
    if frame.color_type == png::ColorType::Rgba {
        // Reuse the decode buffer: no second full-size copy.
        buf.truncate(frame.buffer_size());
        if buf.len() != w * h * 4 { return Err("png: unexpected pixel buffer size".into()); }
        return Ok(Rgba { width: w, height: h, pixels: buf });
    }
    let data = &buf[..frame.buffer_size()];
    let pixels: Vec<u8> = match frame.color_type {
        png::ColorType::Rgba => return Err("png: unexpected colour type".into()),
        png::ColorType::GrayscaleAlpha => data.as_chunks::<2>().0.iter().flat_map(|&[g, a]| [g, g, g, a]).collect(),
        png::ColorType::Rgb => data.as_chunks::<3>().0.iter().flat_map(|&[r, g, b]| [r, g, b, 255]).collect(),
        png::ColorType::Grayscale => data.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return Err("png: palette not expanded".into()),
    };
    if pixels.len() != w * h * 4 {
        return Err("png: unexpected pixel buffer size".into());
    }
    Ok(Rgba { width: w, height: h, pixels })
}

/// Cache `image`, evicting the oldest entries until the entry count and the
/// decoded-byte budget hold. An image larger than the budget is not cached.
fn insert_bounded(cache: &mut Vec<(String, Arc<Rgba>)>, id: &str, image: &Arc<Rgba>) {
    let size = image.pixels.len();
    if size > CACHE_BYTES { return; }
    let mut total: usize = cache.iter().map(|(_, i)| i.pixels.len()).sum();
    while !cache.is_empty() && (cache.len() >= CACHE_ENTRIES || total + size > CACHE_BYTES) {
        total -= cache.remove(0).1.pixels.len();
    }
    cache.push((id.to_owned(), Arc::clone(image)));
}

static STORE: OnceLock<AssetStore> = OnceLock::new();

/// Process-wide store at `STEPHANIA_ASSETS_DIR` (default /var/lib/stephania/assets).
pub fn store() -> &'static AssetStore {
    STORE.get_or_init(|| AssetStore::new(std::env::var("STEPHANIA_ASSETS_DIR").unwrap_or_else(|_| DEFAULT_DIR.into())))
}

/// Ids referenced by a scene's image layers.
pub fn referenced(doc: &serde_json::Value) -> Vec<String> {
    let mut out: Vec<String> = doc["layers"].as_array().into_iter().flatten()
        .filter(|l| l["type"] == "image").filter_map(|l| l["asset"].as_str().map(str::to_owned)).collect();
    out.sort();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn encode_png(w: u32, h: u32, color: png::ColorType, data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, w, h);
            enc.set_color(color);
            enc.set_depth(png::BitDepth::Eight);
            let mut writer = enc.write_header().unwrap();
            writer.write_image_data(data).unwrap();
        }
        out
    }

    fn id_of(bytes: &[u8]) -> String {
        ring::digest::digest(&ring::digest::SHA256, bytes).as_ref().iter().take(8).map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn decodes_colour_types_to_rgba() {
        let rgba = decode_png(&encode_png(2, 1, png::ColorType::Rgba, &[1, 2, 3, 4, 5, 6, 7, 8])).unwrap();
        assert_eq!((rgba.width, rgba.height, rgba.pixels.clone()), (2, 1, vec![1, 2, 3, 4, 5, 6, 7, 8]));
        let rgb = decode_png(&encode_png(1, 1, png::ColorType::Rgb, &[9, 8, 7])).unwrap();
        assert_eq!(rgb.pixels, vec![9, 8, 7, 255]);
        let gray = decode_png(&encode_png(1, 1, png::ColorType::Grayscale, &[50])).unwrap();
        assert_eq!(gray.pixels, vec![50, 50, 50, 255]);
        assert!(decode_png(b"not a png").is_err());
    }

    #[test]
    fn store_checks_id_hash_and_caches() {
        let dir = std::env::temp_dir().join(format!("stephania-assets-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let png = encode_png(1, 1, png::ColorType::Rgba, &[10, 20, 30, 40]);
        let id = id_of(&png);
        std::fs::write(dir.join(format!("{id}.png")), &png).unwrap();
        let store = AssetStore::new(&dir);
        assert_eq!(store.load(&id).unwrap().pixels, vec![10, 20, 30, 40]);
        // Cached: removing the file does not matter any more.
        std::fs::remove_file(dir.join(format!("{id}.png"))).unwrap();
        assert!(store.load(&id).is_ok());
        // A file whose content does not match its name is refused.
        let other = encode_png(1, 1, png::ColorType::Rgba, &[0, 0, 0, 0]);
        std::fs::write(dir.join("0000000000000000.png"), &other).unwrap();
        assert!(store.load("0000000000000000").unwrap_err().contains("does not match"));
        assert!(store.load("../etc/passwd").is_err());
        assert!(store.load("ffffffffffffffff").is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn cache_is_bounded_by_decoded_bytes() {
        let image = |mib: usize| Arc::new(Rgba { width: 1, height: 1, pixels: vec![0; mib * 1024 * 1024] });
        let mut cache = Vec::new();
        for i in 0..4 { insert_bounded(&mut cache, &format!("{i}"), &image(48)); }
        let total: usize = cache.iter().map(|(_, i)| i.pixels.len()).sum();
        assert!(total <= CACHE_BYTES, "{total}");
        assert_eq!(cache.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(), ["2", "3"]);
        insert_bounded(&mut cache, "huge", &image(CACHE_BYTES / 1024 / 1024 + 1));
        assert!(cache.iter().all(|(k, _)| k != "huge"));
        for i in 0..40 { insert_bounded(&mut cache, &format!("s{i}"), &image(0)); }
        assert_eq!(cache.len(), CACHE_ENTRIES);
    }

    #[test]
    fn referenced_ids_are_unique() {
        let doc = serde_json::json!({"layers":[{"type":"image","asset":"b"},{"type":"video"},{"type":"image","asset":"a"},{"type":"image","asset":"b"}]});
        assert_eq!(referenced(&doc), vec!["a".to_string(), "b".to_string()]);
    }
}
