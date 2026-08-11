//! Hand-rolled, dependency-free byte encoders for image output.
//!
//! The core keeps zero image deps, so viewable image bytes are produced here:
//! [`base64_encode`] for data-URL embedding, and [`png_encode`] to wrap raw
//! component samples (the `Raw` image case) as a PNG.
//!
//! PNG uses uncompressed ("stored") DEFLATE blocks via [`zlib_stored`]: no real
//! compressor is needed, just the block framing + an Adler-32 trailer, plus a
//! CRC-32 per chunk. Everything is pure safe arithmetic.

/// Standard base64 (RFC 4648) with `+`/`/` and `=` padding.
pub(crate) fn base64_encode(input: &[u8]) -> String {
    const ALPHA: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((input.len() + 2) / 3 * 4);
    let mut i = 0;
    while i + 3 <= input.len() {
        let n = ((input[i] as u32) << 16) | ((input[i + 1] as u32) << 8) | input[i + 2] as u32;
        out.push(ALPHA[((n >> 18) & 0x3f) as usize] as char);
        out.push(ALPHA[((n >> 12) & 0x3f) as usize] as char);
        out.push(ALPHA[((n >> 6) & 0x3f) as usize] as char);
        out.push(ALPHA[(n & 0x3f) as usize] as char);
        i += 3;
    }
    let rem = input.len() - i;
    if rem == 1 {
        let n = (input[i] as u32) << 16;
        out.push(ALPHA[((n >> 18) & 0x3f) as usize] as char);
        out.push(ALPHA[((n >> 12) & 0x3f) as usize] as char);
        out.push('=');
        out.push('=');
    } else if rem == 2 {
        let n = ((input[i] as u32) << 16) | ((input[i + 1] as u32) << 8);
        out.push(ALPHA[((n >> 18) & 0x3f) as usize] as char);
        out.push(ALPHA[((n >> 12) & 0x3f) as usize] as char);
        out.push(ALPHA[((n >> 6) & 0x3f) as usize] as char);
        out.push('=');
    }
    out
}

/// CRC-32 (PNG/zlib polynomial 0xEDB88320) over one slice.
fn crc32(data: &[u8]) -> u32 {
    crc32_continue(0xFFFF_FFFF, data) ^ 0xFFFF_FFFF
}

/// Continue a CRC-32 from an existing state.
fn crc32_continue(state: u32, data: &[u8]) -> u32 {
    let mut c = state;
    for &b in data {
        c ^= b as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
    }
    c
}

/// Adler-32 over `data` (the zlib trailer).
fn adler32(data: &[u8]) -> u32 {
    const MOD: u32 = 65521;
    let mut a = 1u32;
    let mut b = 0u32;
    for &byte in data {
        a = (a + byte as u32) % MOD;
        b = (b + a) % MOD;
    }
    (b << 16) | a
}

/// Raw DEFLATE stored blocks: BFINAL + LEN + NLEN + bytes (max 65535 per block).
fn deflate_stored(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + (data.len() / 65535 + 1) * 5);
    let mut pos = 0;
    while pos < data.len() {
        let chunk = (data.len() - pos).min(65535);
        out.push(if pos + chunk == data.len() { 1 } else { 0 });
        out.push((chunk & 0xff) as u8);
        out.push((chunk >> 8) as u8);
        let nlen = !(chunk as u16);
        out.push((nlen & 0xff) as u8);
        out.push((nlen >> 8) as u8);
        out.extend_from_slice(&data[pos..pos + chunk]);
        pos += chunk;
    }
    if data.is_empty() {
        out.extend_from_slice(&[1, 0, 0, 0xff, 0xff]);
    }
    out
}

/// A zlib stream wrapping `data` as stored DEFLATE: header + blocks + Adler-32.
fn zlib_stored(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + data.len() / 65535 * 5 + 11);
    out.push(0x78);
    out.push(0x01);
    out.extend(deflate_stored(data));
    let a = adler32(data);
    out.push((a >> 24) as u8);
    out.push((a >> 16) as u8);
    out.push((a >> 8) as u8);
    out.push(a as u8);
    out
}

const PNG_SIG: [u8; 8] = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];

fn samples_per_pixel(color_type: u8) -> Option<u32> {
    match color_type {
        0 => Some(1),
        2 => Some(3),
        _ => None,
    }
}

fn row_bytes(width: u32, spp: u32, bit_depth: u8) -> usize {
    let bits = width as usize * spp as usize * bit_depth as usize;
    (bits + 7) / 8
}

fn valid_png_input(samples: &[u8], width: u32, height: u32, spp: u32, bit_depth: u8) -> bool {
    if width == 0 || height == 0 {
        return false;
    }
    samples.len() == row_bytes(width, spp, bit_depth) * height as usize
}

fn filtered_rows(samples: &[u8], height: usize, rbytes: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(samples.len() + height);
    let mut pos = 0;
    for _ in 0..height {
        out.push(0);
        out.extend_from_slice(&samples[pos..pos + rbytes]);
        pos += rbytes;
    }
    out
}

fn ihdr_data(width: u32, height: u32, bit_depth: u8, color_type: u8) -> Vec<u8> {
    let mut d = Vec::with_capacity(13);
    d.extend_from_slice(&width.to_be_bytes());
    d.extend_from_slice(&height.to_be_bytes());
    d.push(bit_depth);
    d.push(color_type);
    d.push(0);
    d.push(0);
    d.push(0);
    d
}

fn push_chunk(out: &mut Vec<u8>, ctype: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(ctype);
    out.extend_from_slice(data);
    let crc = crc32_continue(crc32_continue(0xFFFF_FFFF, ctype), data) ^ 0xFFFF_FFFF;
    out.extend_from_slice(&crc.to_be_bytes());
}

/// Encode raw component samples as a PNG.  Returns `None` for an unsupported
/// colour type or when `samples` does not exactly fill `width` × `height`.
pub(crate) fn png_encode(
    samples: &[u8],
    width: u32,
    height: u32,
    bit_depth: u8,
    color_type: u8,
) -> Option<Vec<u8>> {
    let spp = samples_per_pixel(color_type)?;
    if !valid_png_input(samples, width, height, spp, bit_depth) {
        return None;
    }
    let rbytes = row_bytes(width, spp, bit_depth);
    let mut out = Vec::with_capacity(samples.len() + height as usize + 128);
    out.extend_from_slice(&PNG_SIG);
    push_chunk(
        &mut out,
        b"IHDR",
        &ihdr_data(width, height, bit_depth, color_type),
    );
    push_chunk(
        &mut out,
        b"IDAT",
        &zlib_stored(&filtered_rows(samples, height as usize, rbytes)),
    );
    push_chunk(&mut out, b"IEND", &[]);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_known_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn crc32_known_vectors() {
        assert_eq!(crc32(b""), 0);
        assert_eq!(crc32(b"a"), 0xe8b7be43);
        assert_eq!(crc32(b"abc"), 0x352441c2);
        assert_eq!(
            crc32(b"The quick brown fox jumps over the lazy dog"),
            0x414fa339
        );
    }

    #[test]
    fn adler32_known_vectors() {
        assert_eq!(adler32(b""), 1);
        assert_eq!(adler32(b"Wikipedia"), 0x11e60398);
        assert_eq!(adler32(b"a"), 0x00620062);
    }

    #[test]
    fn deflate_stored_round_trips_and_framing() {
        let empty = deflate_stored(&[]);
        assert_eq!(empty, vec![1, 0, 0, 0xff, 0xff]);
        let small = deflate_stored(b"abcd");
        assert_eq!(small[0], 1);
        assert_eq!(small[1..3], [4, 0]);
        assert_eq!(small[3..5], [251, 255]);
        assert_eq!(&small[5..], b"abcd");
    }

    #[test]
    fn deflate_stored_splits_at_64k() {
        let data = vec![0xaau8; 70000];
        let def = deflate_stored(&data);
        assert_eq!(def[0], 0);
        let len0 = def[1] as usize | ((def[2] as usize) << 8);
        assert_eq!(len0, 65535);
        assert_eq!(&def[5..5 + 65535], &data[..65535]);
        let tail = &def[5 + 65535..];
        assert_eq!(tail[0], 1);
        let len1 = tail[1] as usize | ((tail[2] as usize) << 8);
        assert_eq!(len1, 70000 - 65535);
    }

    #[test]
    fn zlib_stored_header_and_trailer() {
        let z = zlib_stored(b"abc");
        assert_eq!(&z[0..2], &[0x78, 0x01]);
        // Adler-32 of "abc" is 0x024D0127.
        assert_eq!(&z[z.len() - 4..], &[0x02, 0x4D, 0x01, 0x27]);
    }

    #[test]
    fn png_encode_grayscale_8bit() {
        let png = png_encode(&[0, 128, 200, 255], 2, 2, 8, 0).unwrap();
        assert_eq!(&png[..8], PNG_SIG);
        assert_eq!(&png[8..12], &[0, 0, 0, 13]);
        assert_eq!(&png[12..16], b"IHDR");
        assert_eq!(&png[16..20], 2u32.to_be_bytes().as_slice());
        assert_eq!(&png[20..24], 2u32.to_be_bytes().as_slice());
        assert_eq!(png[24], 8);
        assert_eq!(png[25], 0);
        assert!(png.windows(4).any(|w| w == b"IEND"));
    }

    #[test]
    fn png_encode_rgb_8bit() {
        let png = png_encode(&[255, 0, 0], 1, 1, 8, 2).unwrap();
        assert_eq!(png[25], 2);
        assert!(png.windows(4).any(|w| w == b"IDAT"));
    }

    #[test]
    fn png_encode_grayscale_1bit() {
        let png = png_encode(&[0b10101010], 8, 1, 1, 0).unwrap();
        assert_eq!(png[24], 1);
        assert_eq!(png[25], 0);
        assert!(png.windows(4).any(|w| w == b"IEND"));
    }

    #[test]
    fn png_encode_rejects_bad_input() {
        assert_eq!(png_encode(&[0], 1, 1, 8, 3), None);
        assert_eq!(png_encode(&[0], 0, 1, 8, 0), None);
        assert_eq!(png_encode(&[0], 1, 0, 8, 0), None);
        assert_eq!(png_encode(&[0, 1, 2], 2, 2, 8, 0), None);
    }

    #[test]
    fn png_crc_is_correct() {
        let png = png_encode(&[10, 20, 30, 40], 2, 2, 8, 0).unwrap();
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(b"IHDR");
        ihdr.extend_from_slice(&png[16..29]);
        let want = crc32(&ihdr);
        let got = u32::from_be_bytes([png[29], png[30], png[31], png[32]]);
        assert_eq!(got, want);
    }
}
