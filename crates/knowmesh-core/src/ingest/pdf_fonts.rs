use lopdf::{Dictionary, Document, Encoding, Object, ObjectId};

use crate::error::AppResult;

pub(super) fn prefer_unicode_maps(document: &mut Document) -> AppResult<()> {
    for object in document.objects.values_mut() {
        normalize(object, 0)?;
    }
    Ok(())
}

fn normalize(object: &mut Object, depth: usize) -> AppResult<()> {
    if depth > 64 {
        return Err(super::limit_error());
    }
    match object {
        Object::Dictionary(dictionary) => normalize_dictionary(dictionary, depth)?,
        Object::Stream(stream) => normalize_dictionary(&mut stream.dict, depth)?,
        Object::Array(values) => {
            for value in values {
                normalize(value, depth + 1)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn normalize_dictionary(dictionary: &mut Dictionary, depth: usize) -> AppResult<()> {
    // ToUnicode governs text extraction; lopdf otherwise prefers the glyph encoding.
    if dictionary.has_type(b"Font") && dictionary.has(b"ToUnicode") {
        dictionary.remove(b"Encoding");
    }
    for (_, value) in dictionary.iter_mut() {
        normalize(value, depth + 1)?;
    }
    Ok(())
}

pub(super) fn validate(document: &Document, page: ObjectId, limit: usize) -> lopdf::Result<()> {
    for font in document.get_page_fonts(page)?.values() {
        let encoding = font.get_font_encoding_with_limit(document, limit)?;
        if font.has(b"ToUnicode") {
            if !matches!(encoding, Encoding::UnicodeMapEncoding(_)) {
                return Err(lopdf::Error::CharacterEncoding);
            }
            continue;
        }
        if let Ok(declared) = font.get_deref(b"Encoding", document) {
            match declared {
                Object::Name(name) if name == b"Identity-H" || name == b"Identity-V" => {
                    return Err(lopdf::Error::CharacterEncoding);
                }
                Object::Dictionary(_) if !matches!(encoding, Encoding::Differences(_)) => {
                    return Err(lopdf::Error::CharacterEncoding);
                }
                _ => {}
            }
        } else {
            let name = font.get(b"BaseFont").and_then(Object::as_name)?;
            if ![
                b"Helvetica".as_slice(),
                b"Helvetica-Bold",
                b"Helvetica-Oblique",
                b"Helvetica-BoldOblique",
                b"Times-Roman",
                b"Times-Bold",
                b"Times-Italic",
                b"Times-BoldItalic",
                b"Courier",
                b"Courier-Bold",
                b"Courier-Oblique",
                b"Courier-BoldOblique",
            ]
            .contains(&name)
            {
                return Err(lopdf::Error::CharacterEncoding);
            }
        }
    }
    Ok(())
}
