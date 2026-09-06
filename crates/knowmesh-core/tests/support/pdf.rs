use lopdf::{
    Document, Object, Stream,
    content::{Content, Operation},
    dictionary,
};

pub fn document(pages: &[Option<&[u8]>], replacement_font: bool) -> Document {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let mut font = dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica", "Encoding" => "WinAnsiEncoding" };
    if replacement_font {
        let map = b"/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n/CMapName /Fixture def\n/CMapType 2 def\n1 begincodespacerange\n<00> <FF>\nendcodespacerange\n1 beginbfchar\n<01> <FFFD>\nendbfchar\nendcmap\nCMapName currentdict /CMap defineresource pop\nend end";
        let cmap = doc.add_object(Stream::new(dictionary! {}, map.to_vec()));
        font.set("ToUnicode", cmap);
    }
    let font = doc.add_object(font);
    let image = doc.add_object(Stream::new(dictionary! { "Type" => "XObject", "Subtype" => "Image", "Width" => 1, "Height" => 1, "ColorSpace" => "DeviceRGB", "BitsPerComponent" => 8 }, vec![255, 255, 255]));
    let resources = doc.add_object(dictionary! { "Font" => dictionary! { "F1" => font }, "XObject" => dictionary! { "Im0" => image } });
    let mut kids = Vec::new();
    for text in pages {
        let operations = if let Some(text) = text {
            vec![
                Operation::new("BT", vec![]),
                Operation::new("Tf", vec!["F1".into(), 12.into()]),
                Operation::new("Td", vec![30.into(), 750.into()]),
                Operation::new("Tj", vec![Object::string_literal(text.to_vec())]),
                Operation::new("ET", vec![]),
            ]
        } else {
            vec![
                Operation::new("q", vec![]),
                Operation::new("Do", vec!["Im0".into()]),
                Operation::new("Q", vec![]),
            ]
        };
        let content = Content { operations }.encode().unwrap();
        let stream = doc.add_object(Stream::new(dictionary! {}, content));
        let page = doc.add_object(
            dictionary! { "Type" => "Page", "Parent" => pages_id, "Contents" => stream },
        );
        kids.push(page.into());
    }
    doc.objects.insert(pages_id, dictionary! { "Type" => "Pages", "Kids" => Object::Array(kids), "Count" => pages.len() as i64, "Resources" => resources, "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()] }.into());
    let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    let info =
        doc.add_object(dictionary! { "Title" => Object::string_literal("Synthetic report") });
    doc.trailer.set("Root", catalog);
    doc.trailer.set("Info", info);
    doc.trailer.set(
        "ID",
        vec![
            Object::string_literal("fixture-id"),
            Object::string_literal("fixture-id"),
        ],
    );
    doc
}

pub fn bytes(mut document: Document) -> Vec<u8> {
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).unwrap();
    bytes
}
