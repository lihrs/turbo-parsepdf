//! Standard security handler decryption (ISO 32000-1 §7.6.3 / ISO 32000-2 §7.6.4).
//!
//! Behind the off-by-default `encrypt` feature. Supports the RC4 (V1–V4) and AES
//! (AESV2 = AES-128, AESV3 = AES-256) crypt filters with key revisions R2–R6,
//! using either an empty or a supplied **user or owner** password. The file
//! encryption key is derived once from the `/Encrypt` dictionary; strings and
//! stream bodies are then decrypted per object (RC4/AESV2) or with the file key
//! directly (AESV3). Like the sibling turbo-xlsx `encrypt` module, this is
//! validated functionally (round-trips against qpdf fixtures) and excluded from
//! the line-coverage gate.

use aes::cipher::block_padding::NoPadding;
use aes::cipher::{BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use aes::{Aes128, Aes256};
use md5::{Digest, Md5};
use sha2::{Sha256, Sha384, Sha512};

use crate::object::{Dictionary, Object, Stream};

type Aes128CbcDec = cbc::Decryptor<Aes128>;
type Aes256CbcDec = cbc::Decryptor<Aes256>;
type Aes128CbcEnc = cbc::Encryptor<Aes128>;

/// The 32-byte password padding string (ISO 32000-1 §7.6.3.3, Algorithm 2).
const PAD: [u8; 32] = [
    0x28, 0xBF, 0x4E, 0x5E, 0x4E, 0x75, 0x8A, 0x41, 0x64, 0x00, 0x4E, 0x56, 0xFF, 0xFA, 0x01, 0x08,
    0x2E, 0x2E, 0x00, 0xB6, 0xD0, 0x68, 0x3E, 0x80, 0x2F, 0x0C, 0xA9, 0xFE, 0x64, 0x53, 0x69, 0x7A,
];

/// The per-object cipher of a security handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Method {
    Rc4,
    AesV2,
    AesV3,
}

/// A configured decryptor: the file key + the cipher to apply.
#[derive(Debug, Clone)]
pub struct Decryptor {
    key: Vec<u8>,
    method: Method,
}

impl Decryptor {
    /// Build a decryptor from the `/Encrypt` dictionary, the first file-`/ID`
    /// element, and a user/owner password. Returns `None` for an unsupported or
    /// non-Standard handler.
    pub fn from_encrypt(dict: &Dictionary, id0: &[u8], password: &[u8]) -> Option<Decryptor> {
        if dict.get("Filter").and_then(Object::as_name) != Some("Standard") {
            return None;
        }
        let p = Params::read(dict)?;
        let method = p.method(dict);
        let key = p.derive_key(id0, password)?;
        Some(Decryptor { key, method })
    }

    /// Decrypt one object's bytes (string or raw stream body) at `num`/`generation`.
    pub fn decrypt_bytes(&self, num: u32, generation: u16, data: &[u8]) -> Vec<u8> {
        match self.method {
            Method::Rc4 => rc4(&self.object_key(num, generation, false), data),
            Method::AesV2 => aes128_decrypt(&self.object_key(num, generation, true), data),
            Method::AesV3 => aes256_decrypt(&self.key, data),
        }
    }

    /// Decrypt every string and stream body inside an object.
    pub fn decrypt_object(&self, num: u32, generation: u16, obj: Object) -> Object {
        match obj {
            Object::String(b) => Object::String(self.decrypt_bytes(num, generation, &b)),
            Object::Array(a) => Object::Array(self.decrypt_each(num, generation, a)),
            Object::Dictionary(d) => Object::Dictionary(self.decrypt_dict(num, generation, d)),
            Object::Stream(s) => self.decrypt_stream(num, generation, s),
            other => other,
        }
    }

    fn decrypt_each(&self, num: u32, generation: u16, items: Vec<Object>) -> Vec<Object> {
        items
            .into_iter()
            .map(|o| self.decrypt_object(num, generation, o))
            .collect()
    }

    fn decrypt_dict(&self, num: u32, generation: u16, dict: Dictionary) -> Dictionary {
        let mut out = Dictionary::new();
        for (k, v) in dict.iter() {
            out.insert(k, self.decrypt_object(num, generation, v.clone()));
        }
        out
    }

    fn decrypt_stream(&self, num: u32, generation: u16, s: Stream) -> Object {
        let dict = self.decrypt_dict(num, generation, s.dict);
        let data = self.decrypt_bytes(num, generation, &s.data);
        Object::Stream(Stream { dict, data })
    }

    /// The per-object key (RC4 / AESV2): `MD5(key + num + generation [+ sAlT])`.
    fn object_key(&self, num: u32, generation: u16, aes: bool) -> Vec<u8> {
        let mut h = Md5::new();
        h.update(&self.key);
        h.update(&num.to_le_bytes()[..3]);
        h.update(&generation.to_le_bytes()[..2]);
        if aes {
            h.update(b"sAlT");
        }
        let n = (self.key.len() + 5).min(16);
        h.finalize()[..n].to_vec()
    }
}

/// The numeric fields read from an `/Encrypt` dictionary.
struct Params {
    r: i64,
    length: usize,
    o: Vec<u8>,
    u: Vec<u8>,
    oe: Vec<u8>,
    ue: Vec<u8>,
    p: i64,
}

impl Params {
    fn read(dict: &Dictionary) -> Option<Params> {
        Some(Params {
            r: int(dict, "R")?,
            length: int(dict, "Length").unwrap_or(40) as usize,
            o: bytes(dict, "O")?,
            u: bytes(dict, "U")?,
            oe: bytes(dict, "OE").unwrap_or_default(),
            ue: bytes(dict, "UE").unwrap_or_default(),
            p: int(dict, "P")?,
        })
    }

    /// The cipher implied by `/V` and the `/CF` crypt-filter method.
    fn method(&self, dict: &Dictionary) -> Method {
        match int(dict, "V").unwrap_or(0) {
            5 => Method::AesV3,
            4 => self.cf_method(dict),
            _ => Method::Rc4,
        }
    }

    fn cf_method(&self, dict: &Dictionary) -> Method {
        match cfm(dict).as_deref() {
            Some("AESV3") => Method::AesV3,
            Some("AESV2") => Method::AesV2,
            _ => Method::Rc4,
        }
    }

    /// Derive the file encryption key for the password (R6 or R2–R4).
    fn derive_key(&self, id0: &[u8], password: &[u8]) -> Option<Vec<u8>> {
        if self.r >= 5 {
            self.derive_key_r6(password)
        } else {
            Some(self.derive_key_r4(id0, password))
        }
    }

    /// Algorithm 2 (R2–R4): try `password` as the user password, then as the
    /// owner password (recovering the user password via `/O`), validating each
    /// candidate key against `/U`. Falls back to the user-password key.
    fn derive_key_r4(&self, id0: &[u8], password: &[u8]) -> Vec<u8> {
        let as_user = self.key_from_user_pw(id0, password);
        if self.user_validates(&as_user, id0) {
            return as_user;
        }
        let recovered = self.recover_via_owner(password);
        let as_owner = self.key_from_user_pw(id0, &recovered);
        if self.user_validates(&as_owner, id0) {
            return as_owner;
        }
        as_user
    }

    /// Algorithm 2 proper: the file key for a (candidate) user password.
    fn key_from_user_pw(&self, id0: &[u8], user_pw: &[u8]) -> Vec<u8> {
        let mut h = Md5::new();
        h.update(pad_password(user_pw));
        h.update(&self.o);
        h.update((self.p as i32).to_le_bytes());
        h.update(id0);
        let n = self.length / 8;
        let hash = md5_iterations(h.finalize().to_vec(), n, self.r);
        hash[..n].to_vec()
    }

    /// Recover the user password from `/O` using the owner password (Algorithm 7).
    fn recover_via_owner(&self, owner_pw: &[u8]) -> Vec<u8> {
        let okey = owner_key(owner_pw, self.length / 8, self.r);
        rc4_owner_rounds(&okey, &self.o, self.r)
    }

    /// Validate a file key by recomputing `/U` (Algorithm 4/5) and comparing.
    fn user_validates(&self, key: &[u8], id0: &[u8]) -> bool {
        compute_u(key, id0, self.r).get(..16) == self.u.get(..16)
    }

    /// Algorithm 2.A (R6), AES-256, trying the user then the owner path.
    fn derive_key_r6(&self, password: &[u8]) -> Option<Vec<u8>> {
        self.r6_user(password).or_else(|| self.r6_owner(password))
    }

    fn r6_user(&self, password: &[u8]) -> Option<Vec<u8>> {
        let (vsalt, ksalt) = (self.u.get(32..40)?, self.u.get(40..48)?);
        if hash_r6(password, vsalt, &[]) != self.u.get(0..32)? {
            return None;
        }
        let ik = hash_r6(password, ksalt, &[]);
        Some(aes256_cbc_nopad(&ik, &[0u8; 16], &self.ue))
    }

    fn r6_owner(&self, password: &[u8]) -> Option<Vec<u8>> {
        let u48 = self.u.get(0..48)?;
        let ik = self.r6_owner_key(password, u48)?;
        Some(aes256_cbc_nopad(&ik, &[0u8; 16], &self.oe))
    }

    /// Validate the owner password against `/O` and return its intermediate key.
    fn r6_owner_key(&self, password: &[u8], u48: &[u8]) -> Option<Vec<u8>> {
        let (vsalt, ksalt) = (self.o.get(32..40)?, self.o.get(40..48)?);
        if hash_r6(password, vsalt, u48) != self.o.get(0..32)? {
            return None;
        }
        Some(hash_r6(password, ksalt, u48))
    }
}

/// The `/CF /StdCF /CFM` crypt-filter method name.
fn cfm(dict: &Dictionary) -> Option<String> {
    let cf = dict.get("CF")?.as_dict()?;
    let std = cf.get("StdCF")?.as_dict()?;
    std.get("CFM").and_then(Object::as_name).map(str::to_owned)
}

fn int(dict: &Dictionary, key: &str) -> Option<i64> {
    dict.get(key)?.as_integer()
}

fn bytes(dict: &Dictionary, key: &str) -> Option<Vec<u8>> {
    dict.get(key)?.as_string().map(<[u8]>::to_vec)
}

/// Pad or truncate a password to the 32-byte standard form.
fn pad_password(pw: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    let n = pw.len().min(32);
    out[..n].copy_from_slice(&pw[..n]);
    out[n..].copy_from_slice(&PAD[..32 - n]);
    out
}

/// The 50× MD5 strengthening loop for R≥3 (Algorithm 2 step (h)).
fn md5_iterations(mut hash: Vec<u8>, n: usize, r: i64) -> Vec<u8> {
    if r >= 3 {
        for _ in 0..50 {
            hash = Md5::digest(&hash[..n]).to_vec();
        }
    }
    hash
}

/// The owner RC4 key (Algorithm 3 steps (a)–(d)).
fn owner_key(owner_pw: &[u8], n: usize, r: i64) -> Vec<u8> {
    let mut hash = Md5::digest(pad_password(owner_pw)).to_vec();
    hash = md5_iterations(hash, n, r);
    hash[..n].to_vec()
}

/// Run the RC4 owner rounds over `/O` to recover the user password.
fn rc4_owner_rounds(key: &[u8], o: &[u8], r: i64) -> Vec<u8> {
    if r < 3 {
        return rc4(key, o);
    }
    let mut data = o.to_vec();
    for i in (0..=19u8).rev() {
        let round_key: Vec<u8> = key.iter().map(|&b| b ^ i).collect();
        data = rc4(&round_key, &data);
    }
    data
}

/// Compute `/U` from a file key (Algorithm 4 for R2, Algorithm 5 for R≥3); only
/// the first 16 bytes are significant for validation.
fn compute_u(key: &[u8], id0: &[u8], r: i64) -> Vec<u8> {
    if r < 3 {
        return rc4(key, &PAD);
    }
    let mut h = Md5::new();
    h.update(PAD);
    h.update(id0);
    let mut data = rc4(key, &h.finalize());
    for i in 1..=19u8 {
        let round_key: Vec<u8> = key.iter().map(|&b| b ^ i).collect();
        data = rc4(&round_key, &data);
    }
    data
}

/// RC4 stream cipher (PDFs still use it; RustCrypto dropped it as insecure).
fn rc4(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut s: [u8; 256] = core::array::from_fn(|i| i as u8);
    rc4_ksa(&mut s, key);
    rc4_prga(&mut s, data)
}

fn rc4_ksa(s: &mut [u8; 256], key: &[u8]) {
    let mut j = 0u8;
    for i in 0..256 {
        j = j.wrapping_add(s[i]).wrapping_add(key[i % key.len()]);
        s.swap(i, j as usize);
    }
}

fn rc4_prga(s: &mut [u8; 256], data: &[u8]) -> Vec<u8> {
    let (mut i, mut j) = (0u8, 0u8);
    let mut out = Vec::with_capacity(data.len());
    for &byte in data {
        i = i.wrapping_add(1);
        j = j.wrapping_add(s[i as usize]);
        s.swap(i as usize, j as usize);
        let k = s[(s[i as usize].wrapping_add(s[j as usize])) as usize];
        out.push(byte ^ k);
    }
    out
}

/// AES-128-CBC decrypt with a leading IV and PKCS#7 padding.
fn aes128_decrypt(key: &[u8], data: &[u8]) -> Vec<u8> {
    aes_decrypt(data, |iv, ct| {
        Aes128CbcDec::new_from_slices(key, iv)
            .ok()?
            .decrypt_padded_vec_mut::<Aes128Pad>(ct)
            .ok()
    })
}

/// AES-256-CBC decrypt with a leading IV and PKCS#7 padding.
fn aes256_decrypt(key: &[u8], data: &[u8]) -> Vec<u8> {
    aes_decrypt(data, |iv, ct| {
        Aes256CbcDec::new_from_slices(key, iv)
            .ok()?
            .decrypt_padded_vec_mut::<Aes256Pad>(ct)
            .ok()
    })
}

type Aes128Pad = aes::cipher::block_padding::Pkcs7;
type Aes256Pad = aes::cipher::block_padding::Pkcs7;

/// Split off the 16-byte IV and run a CBC decrypt, falling back to the input on
/// any structural error.
fn aes_decrypt(data: &[u8], f: impl Fn(&[u8], &[u8]) -> Option<Vec<u8>>) -> Vec<u8> {
    match data.split_at_checked(16) {
        Some((iv, ct)) => f(iv, ct).unwrap_or_else(|| data.to_vec()),
        None => data.to_vec(),
    }
}

/// AES-256-CBC decrypt without padding (UE/OE file-key unwrap, IV = zeros).
fn aes256_cbc_nopad(key: &[u8], iv: &[u8], data: &[u8]) -> Vec<u8> {
    Aes256CbcDec::new_from_slices(key, iv)
        .ok()
        .and_then(|c| c.decrypt_padded_vec_mut::<NoPadding>(data).ok())
        .unwrap_or_default()
}

/// AES-128-CBC encrypt without padding (the R6 hash inner round). `data` is
/// always a multiple of the block size (64× a fixed block), so `NoPadding` fits.
fn aes128_cbc_encrypt_nopad(key: &[u8], iv: &[u8], data: &[u8]) -> Vec<u8> {
    match Aes128CbcEnc::new_from_slices(key, iv) {
        Ok(c) => c.encrypt_padded_vec_mut::<NoPadding>(data),
        Err(_) => Vec::new(),
    }
}

/// Algorithm 2.B (R6): the iterated SHA-256/384/512 + AES-128 password hash.
fn hash_r6(password: &[u8], salt: &[u8], udata: &[u8]) -> Vec<u8> {
    let mut k = sha256_of(&[password, salt, udata].concat());
    let mut round = 0usize;
    loop {
        let e = r6_round(password, &k, udata);
        k = r6_next_k(&e);
        round += 1;
        if round >= 64 && usize::from(*e.last().unwrap_or(&0)) <= round - 32 {
            break;
        }
    }
    k[..32].to_vec()
}

/// One R6 round: AES-128-CBC encrypt of 64× `(password‖K‖udata)`.
fn r6_round(password: &[u8], k: &[u8], udata: &[u8]) -> Vec<u8> {
    let block = [password, k, udata].concat();
    let k1: Vec<u8> = block
        .iter()
        .cycle()
        .take(block.len() * 64)
        .copied()
        .collect();
    aes128_cbc_encrypt_nopad(&k[..16], &k[16..32], &k1)
}

/// Pick the next K hash width by `E mod 3` (sum of the first 16 bytes).
fn r6_next_k(e: &[u8]) -> Vec<u8> {
    let m = e.iter().take(16).map(|&b| u32::from(b)).sum::<u32>() % 3;
    match m {
        0 => sha256_of(e),
        1 => Sha384::digest(e).to_vec(),
        _ => Sha512::digest(e).to_vec(),
    }
}

fn sha256_of(data: &[u8]) -> Vec<u8> {
    Sha256::digest(data).to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rc4_known_vector() {
        // RFC 6229 / classic: key "Key", plaintext "Plaintext".
        let ct = rc4(b"Key", b"Plaintext");
        assert_eq!(ct, [0xBB, 0xF3, 0x16, 0xE8, 0xD9, 0x40, 0xAF, 0x0A, 0xD3]);
        // RC4 is symmetric: re-applying recovers the plaintext.
        assert_eq!(rc4(b"Key", &ct), b"Plaintext");
    }

    #[test]
    fn password_padding() {
        assert_eq!(pad_password(b""), PAD);
        let p = pad_password(b"abc");
        assert_eq!(&p[..3], b"abc");
        assert_eq!(&p[3..], &PAD[..29]);
        // Over-long passwords truncate to 32 bytes.
        assert_eq!(pad_password(&[b'x'; 40]), [b'x'; 32]);
    }

    #[test]
    fn aes_short_input_passthrough() {
        // Fewer than 16 bytes → cannot split an IV; returns input unchanged.
        assert_eq!(aes128_decrypt(&[0u8; 16], &[1, 2, 3]), [1, 2, 3]);
    }

    #[test]
    fn owner_rounds_symmetry_r2() {
        // R2 owner rounds are a single RC4 pass.
        let key = b"0123456789abcdef";
        let o = rc4(key, b"useruseruseruser");
        assert_eq!(rc4_owner_rounds(key, &o, 2), b"useruseruseruser");
    }
}
