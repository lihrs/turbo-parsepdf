//! Embedded image extraction (ISO 32000-1 §8.9): image XObjects → raw bytes.
//!
//! A page's `/Resources /XObject` maps names to streams; those with `/Subtype
//! /Image` are pulled out by [`extract_images`]. JPEG (`DCTDecode`) and JPEG2000
//! (`JPXDecode`) bodies are passed through as the original image file bytes;
//! other images (e.g. `FlateDecode`) are decoded to raw component samples and
//! returned with their geometry so a caller can wrap them. Intrinsic size and
//! colour space are reported; on-page placement (the `Do` CTM) is out of scope
//! for v1.

use serde::Serialize;

use crate::filter::decode_stream;
use crate::object::{Dictionary, Object};
use crate::resolver::Resolver;

/// The encoded form of an extracted image's bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ImageFormat {
    /// JPEG (`DCTDecode`): `data` is a complete JPEG file.
    Jpeg,
    /// JPEG 2000 (`JPXDecode`): `data` is a complete JP2 codestream.
    Jpeg2000,
    /// CCITT Group 3/4 fax (`CCITTFaxDecode`).
    Ccitt,
    /// JBIG2 (`JBIG2Decode`).
    Jbig2,
    /// Raw component samples (filters other than the image codecs applied).
    Raw,
}

/// An extracted image XObject.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ParsedImage {
    pub name: String,
    pub format: ImageFormat,
    pub width: u32,
    pub height: u32,
    pub bits_per_component: u32,
    pub color_space: String,
    /// A browser-renderable `data:` URL (JPEG/JPEG2000 passthrough, raw→PNG,
    /// or empty for non-viewable formats). Embedded in HTML/MD/JSON.
    pub data_url: String,
    #[serde(skip)]
    pub data: Vec<u8>,
}

/// Extract every image XObject in a page's `/Resources /XObject` dictionary.
pub fn extract_images(r: &Resolver, resources: &Dictionary) -> Vec<ParsedImage> {
    let mut out = Vec::new();
    if let Some(xobjects) = resources
        .get("XObject")
        .and_then(|x| r.resolve_dict(x).ok())
    {
        for (name, value) in xobjects.iter() {
            push_image(r, &mut out, name, value);
        }
    }
    out
}

/// Append one XObject if it is an image.
fn push_image(r: &Resolver, out: &mut Vec<ParsedImage>, name: &str, value: &Object) {
    if let Some(img) = image_of(r, name, value) {
        out.push(img);
    }
}

/// Build a [`ParsedImage`] from an XObject value, or `None` if it is not an image.
fn image_of(r: &Resolver, name: &str, value: &Object) -> Option<ParsedImage> {
    let stream = r.resolve(value).ok()?;
    let s = stream.as_stream()?;
    if s.dict.get("Subtype").and_then(Object::as_name) != Some("Image") {
        return None;
    }
    let format = classify(&s.dict);
    let data = decode_stream(&s.dict, &s.data).ok()?;
    let width = uint(&s.dict, "Width");
    let height = uint(&s.dict, "Height");
    let bpc = uint_or(&s.dict, "BitsPerComponent", 8);
    let color_space = color_space(r, &s.dict);
    let data_url = image_data_url(format, &color_space, bpc, width, height, &data);
    Some(ParsedImage {
        name: name.to_owned(),
        format,
        width,
        height,
        bits_per_component: bpc,
        color_space,
        data_url,
        data,
    })
}

/// Build a browser-renderable `data:` URL for an image's bytes.
fn image_data_url(
    format: ImageFormat,
    color_space: &str,
    bpc: u32,
    w: u32,
    h: u32,
    data: &[u8],
) -> String {
    match format {
        ImageFormat::Jpeg => data_url_bytes("image/jpeg", data),
        ImageFormat::Jpeg2000 => data_url_bytes("image/jp2", data),
        ImageFormat::Raw => png_data_url(color_space, bpc, w, h, data),
        ImageFormat::Ccitt | ImageFormat::Jbig2 => String::new(),
    }
}

/// A `data:<mime>;base64,<bytes>` URL.
fn data_url_bytes(mime: &str, data: &[u8]) -> String {
    let mut s = String::with_capacity(mime.len() + 13 + (data.len() + 2) / 3 * 4);
    s.push_str("data:");
    s.push_str(mime);
    s.push_str(";base64,");
    s.push_str(&crate::encode::base64_encode(data));
    s
}

/// Transcode raw samples to a PNG `data:` URL, or empty.
fn png_data_url(color_space: &str, bpc: u32, w: u32, h: u32, data: &[u8]) -> String {
    let (bit_depth, color_type) = match png_color_type(color_space, bpc) {
        Some(ct) => ct,
        None => return String::new(),
    };
    match crate::encode::png_encode(data, w, h, bit_depth, color_type) {
        Some(png) => data_url_bytes("image/png", &png),
        None => String::new(),
    }
}

/// Map a PDF colour space + bit depth to a PNG `(bit_depth, colour_type)`.
fn png_color_type(color_space: &str, bpc: u32) -> Option<(u8, u8)> {
    let bpc = bpc as u8;
    match color_space {
        "DeviceGray" | "CalGray" if matches!(bpc, 1 | 2 | 4 | 8 | 16) => Some((bpc, 0)),
        "DeviceRGB" | "CalRGB" if matches!(bpc, 8 | 16) => Some((bpc, 2)),
        _ => None,
    }
}

/// Classify the image format from the last filter in the chain.
fn classify(dict: &Dictionary) -> ImageFormat {
    match last_filter(dict).as_deref() {
        Some("DCTDecode") => ImageFormat::Jpeg,
        Some("JPXDecode") => ImageFormat::Jpeg2000,
        Some("CCITTFaxDecode") => ImageFormat::Ccitt,
        Some("JBIG2Decode") => ImageFormat::Jbig2,
        _ => ImageFormat::Raw,
    }
}

/// The last `/Filter` name (a single name or the last array element).
fn last_filter(dict: &Dictionary) -> Option<String> {
    match dict.get("Filter").or_else(|| dict.get("F"))? {
        Object::Name(n) => Some(n.clone()),
        Object::Array(items) => items.last().and_then(Object::as_name).map(str::to_owned),
        _ => None,
    }
}

/// The image colour space as a readable name (`DeviceRGB`, `ICCBased`, …).
fn color_space(r: &Resolver, dict: &Dictionary) -> String {
    match dict.get("ColorSpace").and_then(|c| r.resolve(c).ok()) {
        Some(Object::Name(n)) => n,
        Some(Object::Array(a)) => array_color_space(&a),
        _ => "DeviceRGB".to_string(),
    }
}

fn array_color_space(items: &[Object]) -> String {
    items
        .first()
        .and_then(Object::as_name)
        .unwrap_or("Unknown")
        .to_string()
}

fn uint(dict: &Dictionary, key: &str) -> u32 {
    uint_or(dict, key, 0)
}

fn uint_or(dict: &Dictionary, key: &str, default: u32) -> u32 {
    dict.get(key)
        .and_then(Object::as_integer)
        .and_then(|n| u32::try_from(n).ok())
        .unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::Document;

    fn assemble(objs: &[String]) -> Vec<u8> {
        let mut pdf = String::from("%PDF-1.5\n");
        let mut offs = Vec::new();
        for (i, body) in objs.iter().enumerate() {
            offs.push(pdf.len());
            pdf.push_str(&format!("{} 0 obj\n{}\nendobj\n", i + 1, body));
        }
        let xoff = pdf.len();
        pdf.push_str(&format!(
            "xref\n0 {}\n0000000000 65535 f \n",
            objs.len() + 1
        ));
        for o in &offs {
            pdf.push_str(&format!("{o:010} 00000 n \n"));
        }
        pdf.push_str(&format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\n",
            objs.len() + 1
        ));
        pdf.push_str(&format!("startxref\n{xoff}\n%%EOF"));
        pdf.into_bytes()
    }

    fn images_for(extra: &[String]) -> Vec<ParsedImage> {
        let mut objs = vec![
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /Resources << /XObject << /Im0 4 0 R /Im1 5 0 R /Fm 6 0 R >> >> >>".to_string(),
        ];
        objs.extend_from_slice(extra);
        let data = assemble(&objs);
        let doc = Document::parse(&data).unwrap();
        let pages = doc.pages().unwrap();
        extract_images(doc.resolver(), &pages[0].resources)
    }

    #[test]
    fn extracts_jpeg_and_raw_images_skips_forms() {
        // The format is classified from /Filter, so a placeholder body suffices
        // to assert DCTDecode passthrough (the real JPEG bytes would just be the
        // stream content, unchanged).
        let jpeg_body = "JPEGSTREAMBYTES";
        let im0 = format!(
            "<< /Type /XObject /Subtype /Image /Width 2 /Height 2 /BitsPerComponent 8 /ColorSpace /DeviceRGB /Filter /DCTDecode /Length {} >>\nstream\n{}\nendstream",
            jpeg_body.len(),
            jpeg_body
        );
        let raw = "ABCD";
        let im1 = format!(
            "<< /Type /XObject /Subtype /Image /Width 2 /Height 2 /ColorSpace /DeviceGray /Length {} >>\nstream\n{}\nendstream",
            raw.len(),
            raw
        );
        // A Form XObject must be ignored.
        let fm = "<< /Type /XObject /Subtype /Form /Length 0 >>\nstream\n\nendstream";
        let imgs = images_for(&[im0, im1, fm.to_string()]);
        assert_eq!(imgs.len(), 2);
        let jpeg = imgs.iter().find(|i| i.name == "Im0").unwrap();
        assert_eq!(jpeg.format, ImageFormat::Jpeg);
        assert_eq!((jpeg.width, jpeg.height), (2, 2));
        assert_eq!(jpeg.color_space, "DeviceRGB");
        assert_eq!(jpeg.data, b"JPEGSTREAMBYTES"); // DCTDecode passthrough
        let raw_img = imgs.iter().find(|i| i.name == "Im1").unwrap();
        assert_eq!(raw_img.format, ImageFormat::Raw);
        assert_eq!(raw_img.bits_per_component, 8);
        assert_eq!(raw_img.data, b"ABCD");
        let expect_jpeg = format!(
            "data:image/jpeg;base64,{}",
            crate::encode::base64_encode(b"JPEGSTREAMBYTES")
        );
        assert_eq!(jpeg.data_url, expect_jpeg);
        let png = crate::encode::png_encode(b"ABCD", 2, 2, 8, 0).unwrap();
        let expect_raw = format!(
            "data:image/png;base64,{}",
            crate::encode::base64_encode(&png)
        );
        assert_eq!(raw_img.data_url, expect_raw);
    }

    #[test]
    fn image_data_url_branches() {
        assert!(
            image_data_url(ImageFormat::Jpeg, "DeviceRGB", 8, 1, 1, b"\xff\xd8\xff")
                .starts_with("data:image/jpeg;base64,")
        );
        assert!(
            image_data_url(ImageFormat::Jpeg2000, "DeviceRGB", 8, 1, 1, b"\x00\x00")
                .starts_with("data:image/jp2;base64,")
        );
        assert!(
            image_data_url(ImageFormat::Raw, "DeviceGray", 8, 2, 2, &[0, 1, 2, 3])
                .starts_with("data:image/png;base64,")
        );
        assert!(
            image_data_url(ImageFormat::Raw, "DeviceRGB", 8, 1, 1, &[10, 20, 30])
                .starts_with("data:image/png;base64,")
        );
        assert_eq!(
            image_data_url(ImageFormat::Raw, "ICCBased", 8, 1, 1, &[0]),
            ""
        );
        assert_eq!(
            image_data_url(ImageFormat::Raw, "DeviceRGB", 8, 1, 1, &[10, 20]),
            ""
        );
        assert_eq!(
            image_data_url(ImageFormat::Ccitt, "DeviceGray", 1, 1, 1, &[0]),
            ""
        );
        assert_eq!(
            image_data_url(ImageFormat::Jbig2, "DeviceGray", 1, 1, 1, &[0]),
            ""
        );
    }

    #[test]
    fn png_color_type_mapping() {
        assert_eq!(png_color_type("DeviceGray", 8), Some((8, 0)));
        assert_eq!(png_color_type("CalGray", 1), Some((1, 0)));
        assert_eq!(png_color_type("DeviceRGB", 8), Some((8, 2)));
        assert_eq!(png_color_type("CalRGB", 16), Some((16, 2)));
        assert_eq!(png_color_type("DeviceGray", 7), None);
        assert_eq!(png_color_type("DeviceRGB", 4), None);
        assert_eq!(png_color_type("ICCBased", 8), None);
        assert_eq!(png_color_type("DeviceCMYK", 8), None);
    }

    #[test]
    fn classify_and_colorspace_helpers() {
        let mut d = Dictionary::new();
        d.insert(
            "Filter",
            Object::Array(vec![Object::Name("JPXDecode".into())]),
        );
        assert_eq!(classify(&d), ImageFormat::Jpeg2000);
        let mut d2 = Dictionary::new();
        d2.insert("Filter", Object::Name("CCITTFaxDecode".into()));
        assert_eq!(classify(&d2), ImageFormat::Ccitt);
        let mut d3 = Dictionary::new();
        d3.insert("Filter", Object::Name("JBIG2Decode".into()));
        assert_eq!(classify(&d3), ImageFormat::Jbig2);
        // No filter → Raw.
        assert_eq!(classify(&Dictionary::new()), ImageFormat::Raw);
        // A non-name filter entry yields no last filter.
        let mut d4 = Dictionary::new();
        d4.insert("Filter", Object::Integer(1));
        assert_eq!(classify(&d4), ImageFormat::Raw);
    }

    #[test]
    fn array_colorspace_name() {
        let icc = Object::Array(vec![Object::Name("ICCBased".into()), Object::Integer(0)]);
        assert_eq!(array_color_space(icc.as_array().unwrap()), "ICCBased");
        assert_eq!(array_color_space(&[]), "Unknown");
    }

    #[test]
    fn image_with_array_colorspace() {
        // /ColorSpace as an array (ICCBased) → its leading name is reported.
        let im0 = "<< /Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace [/ICCBased 7 0 R] /Length 1 >>\nstream\nZ\nendstream";
        let imgs = images_for(&[im0.to_string()]);
        assert_eq!(imgs[0].color_space, "ICCBased");
        // An image with no /ColorSpace falls back to DeviceRGB.
        let im1 = "<< /Type /XObject /Subtype /Image /Width 1 /Height 1 /Length 1 >>\nstream\nZ\nendstream";
        let imgs = images_for(&[im0.to_string(), im1.to_string()]);
        assert_eq!(imgs[1].color_space, "DeviceRGB");
        assert_eq!(imgs[0].data_url, "");
        assert_eq!(imgs[1].data_url, "");
    }

    #[test]
    fn no_xobject_resource_is_empty() {
        let data = assemble(&[
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R >>".to_string(),
        ]);
        let doc = Document::parse(&data).unwrap();
        let pages = doc.pages().unwrap();
        assert!(extract_images(doc.resolver(), &pages[0].resources).is_empty());
    }
}
