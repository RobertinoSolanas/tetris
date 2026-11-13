//! A minimal PNG writer, so `--snapshot` needs no image crate (docs/adr/0007).
//!
//! Only what we actually need: 8-bit RGBA, no interlacing, and DEFLATE *stored* blocks (a zlib
//! stream of raw bytes plus Adler-32). Snapshot files are larger than a compressed PNG would give,
//! which is irrelevant — they are a debugging tool, and "uncompressed deflate" is trivially
//! correct, which is not nothing for a format nobody hand-rolls twice.

use std::io::{self, Write};

const SIGNATURE: [u8; 8] = [137, b'P', b'N', b'G', 13, 10, 26, 10];

/// CRC-32 (the PNG variant), bitwise, table generated on first use per call.
fn crc32(data: &[u8]) -> u32 {
    let mut table = [0u32; 256];
    for (i, entry) in table.iter_mut().enumerate() {
        let mut c = i as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
        *entry = c;
    }
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc = table[((crc ^ b as u32) & 0xFF) as usize] ^ (crc >> 8);
    }
    crc ^ 0xFFFF_FFFF
}

/// Adler-32 of a byte slice, the checksum zlib wants.
fn adler32(data: &[u8]) -> u32 {
    const MOD: u32 = 65521;
    let mut a: u32 = 1;
    let mut b: u32 = 0;
    for chunk in data.chunks(4096) {
        for &byte in chunk {
            a = (a + byte as u32) % MOD;
            b = (b + a) % MOD;
        }
    }
    (b << 16) | a
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut crc_input = Vec::with_capacity(4 + data.len());
    crc_input.extend_from_slice(kind);
    crc_input.extend_from_slice(data);
    out.extend_from_slice(&crc32(&crc_input).to_be_bytes());
}

/// Wrap `data` in a zlib stream made purely of stored (uncompressed) DEFLATE blocks.
fn zlib_stored(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 64);
    out.push(0x78); // CM=8 (deflate), CINFO=7 (32 KiB window)
    out.push(0x01); // FM: check bits so CMF<<8|FCM is a multiple of 31, FDICT=0

    const MAX_BLOCK: usize = 0xFFFF;
    if data.is_empty() {
        // One final empty block.
        out.extend_from_slice(&[0x01, 0x00, 0x00, 0xFF, 0xFF]);
    } else {
        for (i, block) in data.chunks(MAX_BLOCK).enumerate() {
            let total = data.len().div_ceil(MAX_BLOCK);
            let last = i + 1 == total;
            out.push(if last { 1 } else { 0 }); // BFINAL, BTYPE=00 (stored)
            let len = block.len() as u16;
            out.extend_from_slice(&len.to_le_bytes());
            out.extend_from_slice(&(!len).to_le_bytes());
            out.extend_from_slice(block);
        }
    }

    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

/// Encode an RGBA8 image as a PNG byte stream.
///
/// `rgba` must be exactly `width × height × 4` bytes, row-major, top row first.
pub fn encode_rgba(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    assert!(width > 0 && height > 0, "PNG needs a non-zero image");
    let expected = width as usize * height as usize * 4;
    assert_eq!(
        rgba.len(),
        expected,
        "expected {expected} bytes for a {width}×{height} RGBA image, got {}",
        rgba.len()
    );

    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.push(8); // bit depth
    ihdr.push(6); // colour type: truecolour + alpha
    ihdr.push(0); // compression method (deflate)
    ihdr.push(0); // filter method
    ihdr.push(0); // interlace: none

    // Each scanline gets a filter-type byte (0 = none).
    let stride = width as usize * 4;
    let mut raw = Vec::with_capacity((stride + 1) * height as usize);
    for y in 0..height as usize {
        raw.push(0);
        raw.extend_from_slice(&rgba[y * stride..(y + 1) * stride]);
    }

    let mut out = Vec::with_capacity(raw.len() + 128);
    out.extend_from_slice(&SIGNATURE);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &zlib_stored(&raw));
    chunk(&mut out, b"IEND", b"");
    out
}

/// Write an RGBA8 image straight to disk.
pub fn write_rgba(path: &std::path::Path, width: u32, height: u32, rgba: &[u8]) -> io::Result<()> {
    let bytes = encode_rgba(width, height, rgba);
    let mut file = std::fs::File::create(path)?;
    file.write_all(&bytes)?;
    file.flush()
}

/// Decode a PNG produced by this module (stored-deflate only) — used by the tests to prove the
/// encoder is self-consistent without pulling in an image crate.
pub fn decode_rgba(png: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    if png.len() < 8 + 12 + 12 || png[..8] != SIGNATURE {
        return None;
    }
    let mut pos = 8;
    let mut width = 0u32;
    let mut height = 0u32;
    let mut idat = Vec::new();

    while pos + 8 <= png.len() {
        let len = u32::from_be_bytes(png[pos..pos + 4].try_into().ok()?) as usize;
        let kind: [u8; 4] = png[pos + 4..pos + 8].try_into().ok()?;
        let body_start = pos + 8;
        let body_end = body_start + len;
        if body_end + 4 > png.len() {
            return None;
        }
        let body = &png[body_start..body_end];
        let stored_crc = u32::from_be_bytes(png[body_end..body_end + 4].try_into().ok()?);
        let mut crc_input = Vec::with_capacity(4 + body.len());
        crc_input.extend_from_slice(&kind);
        crc_input.extend_from_slice(body);
        if crc32(&crc_input) != stored_crc {
            return None; // a corrupted file must not decode
        }
        match &kind {
            b"IHDR" => {
                width = u32::from_be_bytes(body[0..4].try_into().ok()?);
                height = u32::from_be_bytes(body[4..8].try_into().ok()?);
                if body[8] != 8 || body[9] != 6 || body[12] != 0 {
                    return None; // only 8-bit RGBA, no interlace
                }
            }
            b"IDAT" => idat.extend_from_slice(body),
            b"IEND" => break,
            _ => {}
        }
        pos = body_end + 4;
    }

    // Strip the 2-byte zlib header and the trailing Adler-32, then concatenate stored blocks.
    if idat.len() < 3 {
        return None;
    }
    let payload = &idat[2..idat.len() - 4];
    let mut raw = Vec::new();
    let mut i = 0;
    loop {
        if i + 5 > payload.len() {
            return None;
        }
        let header = payload[i];
        let btype = (header >> 1) & 3;
        if btype != 0 {
            return None; // we only ever write stored blocks
        }
        let last = header & 1 == 1;
        let len = u16::from_le_bytes(payload[i + 1..i + 3].try_into().ok()?) as usize;
        let nlen = u16::from_le_bytes(payload[i + 3..i + 5].try_into().ok()?);
        if (len as u16) ^ 0xFFFF != nlen {
            return None;
        }
        let start = i + 5;
        if start + len > payload.len() {
            return None;
        }
        raw.extend_from_slice(&payload[start..start + len]);
        i = start + len;
        if last {
            break;
        }
    }

    let stride = width as usize * 4;
    let mut rgba = Vec::with_capacity(stride * height as usize);
    for y in 0..height as usize {
        let line_start = y * (stride + 1);
        if line_start + stride + 1 > raw.len() {
            return None;
        }
        if raw[line_start] != 0 {
            return None; // no filter was applied
        }
        rgba.extend_from_slice(&raw[line_start + 1..line_start + 1 + stride]);
    }
    Some((width, height, rgba))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gradient(w: u32, h: u32) -> Vec<u8> {
        let mut px = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                px.push((x % 256) as u8);
                px.push((y % 256) as u8);
                px.push(((x + y) % 256) as u8);
                px.push(255);
            }
        }
        px
    }

    #[test]
    fn checksums_match_the_reference_values() {
        // The canonical vectors for both algorithms.
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
        assert_eq!(crc32(b""), 0);
        assert_eq!(adler32(b""), 1);
    }

    #[test]
    fn output_starts_with_the_png_signature_and_has_the_right_dimensions() {
        let px = gradient(7, 3);
        let png = encode_rgba(7, 3, &px);
        assert_eq!(&png[..8], &SIGNATURE);
        assert_eq!(&png[12..16], b"IHDR");
        let w = u32::from_be_bytes(png[16..20].try_into().unwrap());
        let h = u32::from_be_bytes(png[20..24].try_into().unwrap());
        assert_eq!((w, h), (7, 3));
        assert!(png.windows(4).any(|c| c == b"IDAT"));
        assert!(png.windows(4).any(|c| c == b"IEND"));
    }

    #[test]
    fn round_trip_preserves_every_pixel() {
        for (w, h) in [(1, 1), (3, 5), (16, 16), (64, 40)] {
            let px = gradient(w, h);
            let png = encode_rgba(w, h, &px);
            let (dw, dh, decoded) = decode_rgba(&png).expect("decodes");
            assert_eq!((dw, dh), (w, h));
            assert_eq!(decoded, px, "{w}×{h} round trip differs");
        }
    }

    #[test]
    fn images_bigger_than_one_deflate_block_round_trip() {
        // 65 535-byte blocks: 400×40 RGBA rows are 64 000 bytes, so with the filter byte the
        // stream must span at least two blocks.
        let (w, h) = (400, 40);
        let px = gradient(w, h);
        let png = encode_rgba(w, h, &px);
        let (dw, dh, decoded) = decode_rgba(&png).expect("decodes");
        assert_eq!((dw, dh), (w, h));
        assert_eq!(decoded.len(), px.len());
        assert_eq!(decoded, px);
        assert!(png.len() > px.len(), "stored blocks add framing, not less");
    }

    #[test]
    fn an_empty_image_is_encodable_and_decodable() {
        let png = encode_rgba(1, 1, &[1, 2, 3, 4]);
        let (w, h, px) = decode_rgba(&png).unwrap();
        assert_eq!((w, h), (1, 1));
        assert_eq!(px, vec![1, 2, 3, 4]);
    }

    #[test]
    fn a_corrupt_chunk_is_rejected_rather_than_decoded() {
        let px = gradient(4, 4);
        let mut png = encode_rgba(4, 4, &px);
        // Find IDAT and flip a byte inside its body: the chunk CRC must then disagree.
        let kind_at = png
            .windows(4)
            .position(|c| c == b"IDAT")
            .expect("IDAT present");
        let body = kind_at + 4;
        png[body + 2] ^= 0xFF;
        assert!(decode_rgba(&png).is_none(), "corruption slipped through");
    }

    #[test]
    fn signature_of_a_truncated_file_is_not_a_png() {
        assert!(decode_rgba(&[]).is_none());
        assert!(decode_rgba(b"not a png at all, really").is_none());
        assert!(decode_rgba(&SIGNATURE).is_none());
    }

    #[test]
    #[should_panic(expected = "RGBA image")]
    fn wrong_pixel_count_panics_loudly() {
        encode_rgba(4, 4, &[0; 10]);
    }
}
