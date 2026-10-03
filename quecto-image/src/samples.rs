//! Real minimal image files for tests (#2420, #2422): each builder writes a
//! header a decoder accepts (with its CRC where the format has one) and a
//! little body after it; [`crate::encode`] gives the base64 a conversation
//! carries. Built only for tests and the `test-support` feature.

/// Base64 as a conversation carries it: strict standard, one line.
pub fn encode(bytes: &[u8]) -> String {
    crate::encode(bytes)
}

/// `len` deterministic bytes that do not compress: an image body.
pub fn noise(len: usize, seed: u64) -> Vec<u8> {
    let mut state = seed;
    (0..len)
        .map(|_| {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1);
            (state >> 33) as u8
        })
        .collect()
}

/// The CRC-32 a PNG chunk carries (ISO-HDLC, reflected 0xEDB88320).
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xEDB8_8320 & (crc & 1).wrapping_neg());
        }
    }
    !crc
}

fn png_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&u32::try_from(data.len()).unwrap().to_be_bytes());
    let start = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let crc = crc32(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

/// A PNG: signature, IHDR (8-bit RGBA), an IDAT of `body` bytes, IEND.
pub fn png_with_body(width: u32, height: u32, body: usize) -> Vec<u8> {
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    png_chunk(&mut out, b"IHDR", &ihdr);
    png_chunk(&mut out, b"IDAT", &noise(body, u64::from(width ^ height)));
    png_chunk(&mut out, b"IEND", &[]);
    out
}

pub fn png(width: u32, height: u32) -> Vec<u8> {
    png_with_body(width, height, 16)
}

fn jpeg_segment(out: &mut Vec<u8>, marker: u8, data: &[u8]) {
    out.extend_from_slice(&[0xFF, marker]);
    out.extend_from_slice(&u16::try_from(data.len() + 2).unwrap().to_be_bytes());
    out.extend_from_slice(data);
}

/// A baseline JPEG: SOI, APP0 JFIF, an APP1 of `exif` bytes (a camera's
/// EXIF block and thumbnail sit there, before the frame), DQT, SOF0 (3
/// components), DHT, SOS, scan data, EOI. `sof` is the frame marker.
pub fn jpeg_with(width: u16, height: u16, exif: usize, sof: u8) -> Vec<u8> {
    let mut out = vec![0xFF, 0xD8];
    jpeg_segment(
        &mut out,
        0xE0,
        b"JFIF\0\x01\x01\x00\x00\x01\x00\x01\x00\x00",
    );
    if exif > 0 {
        let mut app1 = b"Exif\0\0".to_vec();
        app1.extend(noise(exif, 1));
        jpeg_segment(&mut out, 0xE1, &app1);
    }
    let mut dqt = vec![0x00];
    dqt.extend(1..=64u8);
    jpeg_segment(&mut out, 0xDB, &dqt);
    let mut sof_data = vec![8];
    sof_data.extend_from_slice(&height.to_be_bytes());
    sof_data.extend_from_slice(&width.to_be_bytes());
    sof_data.extend_from_slice(&[3, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1]);
    jpeg_segment(&mut out, sof, &sof_data);
    let mut dht = vec![0x00, 0, 1];
    dht.extend([0; 14]);
    dht.push(0);
    jpeg_segment(&mut out, 0xC4, &dht);
    jpeg_segment(&mut out, 0xDA, &[3, 1, 0x00, 2, 0x11, 3, 0x11, 0, 63, 0]);
    out.extend(noise(32, 2).into_iter().map(|b| b & 0x7F));
    out.extend_from_slice(&[0xFF, 0xD9]);
    out
}

pub fn jpeg(width: u16, height: u16) -> Vec<u8> {
    jpeg_with(width, height, 0, 0xC0)
}

/// A GIF89a: header, logical screen descriptor with a 2-colour global
/// table, one image descriptor, a tiny LZW block, trailer.
pub fn gif(width: u16, height: u16) -> Vec<u8> {
    let mut out = b"GIF89a".to_vec();
    out.extend_from_slice(&width.to_le_bytes());
    out.extend_from_slice(&height.to_le_bytes());
    out.extend_from_slice(&[0x80, 0, 0]);
    out.extend_from_slice(&[0, 0, 0, 0xFF, 0xFF, 0xFF]);
    out.push(0x2C);
    out.extend_from_slice(&[0, 0, 0, 0]);
    out.extend_from_slice(&width.to_le_bytes());
    out.extend_from_slice(&height.to_le_bytes());
    out.push(0);
    out.extend_from_slice(&[2, 2, 0x4C, 0x01, 0]);
    out.push(0x3B);
    out
}

fn riff_webp(chunks: &[u8]) -> Vec<u8> {
    let mut out = b"RIFF".to_vec();
    out.extend_from_slice(&u32::try_from(chunks.len() + 4).unwrap().to_le_bytes());
    out.extend_from_slice(b"WEBP");
    out.extend_from_slice(chunks);
    out
}

fn webp_chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut out = kind.to_vec();
    out.extend_from_slice(&u32::try_from(data.len()).unwrap().to_le_bytes());
    out.extend_from_slice(data);
    if data.len() % 2 == 1 {
        out.push(0);
    }
    out
}

/// A lossy WebP ("VP8 "): a key-frame tag, the start code, 14-bit sides
/// (scale bits 0), then partition bytes. Sides are 1..=16383.
pub fn webp_lossy(width: u16, height: u16) -> Vec<u8> {
    let mut frame = vec![0x50, 0x02, 0x00, 0x9D, 0x01, 0x2A];
    frame.extend_from_slice(&width.to_le_bytes());
    frame.extend_from_slice(&height.to_le_bytes());
    frame.extend(noise(24, 3));
    riff_webp(&webp_chunk(b"VP8 ", &frame))
}

fn vp8l(width: u32, height: u32) -> Vec<u8> {
    let bits = (width - 1) | ((height - 1) << 14) | (1 << 28);
    let mut data = vec![0x2F];
    data.extend_from_slice(&bits.to_le_bytes());
    data.extend(noise(12, 4));
    webp_chunk(b"VP8L", &data)
}

/// A lossless WebP ("VP8L"): signature 0x2F, 14-bit sides minus one,
/// alpha bit, version 0. Sides are 1..=16384.
pub fn webp_lossless(width: u32, height: u32) -> Vec<u8> {
    riff_webp(&vp8l(width, height))
}

/// An extended WebP ("VP8X"): flags, 24-bit canvas sides minus one, then
/// a lossless image chunk. Canvas sides are 1..=16777216.
pub fn webp_extended(width: u32, height: u32) -> Vec<u8> {
    let mut data = vec![0x10, 0, 0, 0];
    data.extend_from_slice(&(width - 1).to_le_bytes()[..3]);
    data.extend_from_slice(&(height - 1).to_le_bytes()[..3]);
    let mut chunks = webp_chunk(b"VP8X", &data);
    chunks.extend(vp8l(width.min(16_384), height.min(16_384)));
    riff_webp(&chunks)
}

/// A GIF89a of `frames` 1x1 frames, each after a graphic control extension,
/// with a global colour table and a comment extension (#2421): animated
/// when `frames` is more than one.
pub fn animated_gif(frames: usize) -> Vec<u8> {
    let mut bytes = b"GIF89a".to_vec();
    bytes.extend([1, 0, 1, 0, 0x80, 0, 0]); // 1x1, a 2-colour global table
    bytes.extend([0, 0, 0, 255, 255, 255]);
    bytes.extend([0x21, 0xFE, 3, b'h', b'e', b'y', 0]); // comment
    for _ in 0..frames {
        bytes.extend([0x21, 0xF9, 4, 0, 10, 0, 0, 0]); // graphic control
        bytes.extend([0x2C, 0, 0, 0, 0, 1, 0, 1, 0, 0]); // image descriptor
        bytes.extend([2, 2, 0x4C, 0x01, 0]); // LZW size, one sub-block, end
    }
    bytes.push(0x3B);
    bytes
}

/// A 1x1 image of `mime`.
pub fn sample(mime: crate::ImageMime) -> Vec<u8> {
    match mime {
        crate::ImageMime::Png => png(1, 1),
        crate::ImageMime::Jpeg => jpeg(1, 1),
        crate::ImageMime::Gif => gif(1, 1),
        crate::ImageMime::Webp => webp_lossless(1, 1),
    }
}
