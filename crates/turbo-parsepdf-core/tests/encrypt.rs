//! Decryption round-trips against qpdf-encrypted fixtures (RC4, AES-128, AES-256,
//! plus user/owner-password files). Runs only with `--features encrypt`.
#![cfg(feature = "encrypt")]

use turbo_parsepdf_core::Document;

const RC4: &[u8] = include_bytes!("fixtures/rc4.pdf");
const AES128: &[u8] = include_bytes!("fixtures/aes128.pdf");
const AES256: &[u8] = include_bytes!("fixtures/aes256.pdf");
const PW_AES256: &[u8] = include_bytes!("fixtures/pw_aes256.pdf");
const PW_RC4: &[u8] = include_bytes!("fixtures/pw_rc4.pdf");

fn extract_text(data: &[u8], password: &str) -> String {
    let doc = Document::parse_with_password(data, password).expect("parse");
    let extracted = doc.extract().expect("extract");
    extracted
        .pages
        .iter()
        .map(|p| p.text())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn rc4_empty_password() {
    assert!(extract_text(RC4, "").contains("turbo-parsepdf"));
}

#[test]
fn aes128_empty_password() {
    assert!(extract_text(AES128, "").contains("turbo-parsepdf"));
}

#[test]
fn aes256_empty_password() {
    let text = extract_text(AES256, "");
    assert!(text.contains("turbo-parsepdf"));
    assert!(text.contains("Second line"));
}

#[test]
fn aes256_user_password() {
    assert!(extract_text(PW_AES256, "secret").contains("turbo-parsepdf"));
}

#[test]
fn aes256_owner_password() {
    // The owner password unlocks the same content via the owner key path.
    assert!(extract_text(PW_AES256, "owner").contains("turbo-parsepdf"));
}

#[test]
fn rc4_user_password() {
    assert!(extract_text(PW_RC4, "pw123").contains("turbo-parsepdf"));
}
