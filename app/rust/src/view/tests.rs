use super::markdown::{parse, BlockKind};
use super::*;
use crate::api::viewer::PreviewKind;
use crate::crypto::password::Argon2Cost;
use crate::vault::Vault;

fn png(w: u32, h: u32) -> Vec<u8> {
    let img = RgbaImage::from_fn(w, h, |x, y| image::Rgba([x as u8, y as u8, 200, 255]));
    let mut out = Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

/// A one-page PDF (200×100 points) with a red rectangle on the left half.
fn pdf() -> Vec<u8> {
    pdf_with(b"1 0 0 rg 0 0 100 100 re f")
}

fn pdf_with(content: &[u8]) -> Vec<u8> {
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 100] /Contents 4 0 R \
         /Resources << /Font << /F1 5 0 R >> >> >>"
            .to_string(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            std::str::from_utf8(content).unwrap()
        ),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
    ];
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for off in offsets {
        out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    out
}

#[test]
fn names_and_sniffing() {
    assert_eq!(preview_for_name("Photo.JPG"), Preview::Image);
    assert_eq!(preview_for_name("notes.md"), Preview::Markdown);
    assert_eq!(preview_for_name("main.rs"), Preview::Text);
    assert_eq!(preview_for_name("README"), Preview::Text);
    assert_eq!(preview_for_name("archive.zip"), Preview::None);
    assert_eq!(preview_for_name(".png"), Preview::None);
    assert_eq!(describe("a.pdf"), "PDF document");
    assert_eq!(describe("a.xyz"), "XYZ file");

    // Content wins over the name.
    assert_eq!(sniff("not-a-picture.txt", &png(2, 2)), Preview::Image);
    assert_eq!(sniff("scan", &pdf()), Preview::Pdf);
    assert_eq!(sniff("fake.png", b"hello there"), Preview::Text);
    assert_eq!(sniff("data.txt", &[0u8, 1, 2, 3, 0, 9]), Preview::None);
    assert_eq!(sniff("empty.bin", b""), Preview::None);
}

#[test]
fn text_decoding() {
    let (t, cut) = decode_text(b"\xef\xbb\xbfcaf\xc3\xa9\r\nline\ttwo");
    assert_eq!(t.as_str(), "café\r\nline\ttwo");
    assert!(!cut);
    assert_eq!(lines(&t), vec!["café", "line    two"]);

    let utf16: Vec<u8> = [0xff, 0xfe]
        .into_iter()
        .chain("hi ✓".encode_utf16().flat_map(|u| u.to_le_bytes()))
        .collect();
    assert_eq!(decode_text(&utf16).0.as_str(), "hi ✓");

    let big = vec![b'a'; MAX_TEXT_BYTES + 10];
    let (t, cut) = decode_text(&big);
    assert!(cut);
    assert_eq!(t.len(), MAX_TEXT_BYTES);
}

#[test]
fn images_decode_and_scale() {
    let d = decode_image(&png(400, 200)).unwrap();
    assert_eq!((d.width(), d.height()), (400, 200));
    let p = d.render(100, 100);
    assert_eq!((p.width, p.height), (100, 50));
    assert_eq!(p.rgba.len(), 100 * 50 * 4);
    // Never enlarged.
    let p = d.render(1000, 1000);
    assert_eq!((p.width, p.height), (400, 200));
    assert_eq!(&p.rgba[..4], &[0, 0, 200, 255]);
    assert!(decode_image(b"\x89PNG\r\n\x1a\nbroken").is_err());
}

#[test]
fn pdf_renders_pages() {
    let doc = PdfDoc::open(Zeroizing::new(pdf())).unwrap();
    assert_eq!(doc.page_sizes(), vec![(200.0, 100.0)]);
    let p = doc.render(0, 400, 400).unwrap();
    assert_eq!((p.width, p.height), (400, 200));
    let px = |x: u32, y: u32| {
        let i = ((y * p.width + x) * 4) as usize;
        p.rgba[i..i + 4].to_vec()
    };
    assert_eq!(px(50, 100), vec![255, 0, 0, 255], "left half is red");
    assert_eq!(
        px(350, 100),
        vec![255, 255, 255, 255],
        "right half is white"
    );
    assert!(doc.render(1, 100, 100).is_err());
    assert!(PdfDoc::open(Zeroizing::new(b"%PDF-1.4 nonsense".to_vec())).is_err());
}

#[test]
fn markdown_blocks() {
    let md = "# Title\n\nSome **bold** and *it* with `code` and [a link](https://x.y).\n\n\
              - one\n- two\n  1. inner\n\n- [x] done\n\n> quoted\n\n```\nfn main() {}\n```\n\n---\n\n\
              | a | b |\n|---|---|\n| 1 | 2 |\n\n![cat](cat.png)\n\n<b>raw</b>\n";
    let b = parse(md);
    let kinds: Vec<_> = b.iter().map(|b| b.kind).collect();
    assert_eq!(
        kinds,
        vec![
            BlockKind::Heading,
            BlockKind::Paragraph,
            BlockKind::Bullet,
            BlockKind::Bullet,
            BlockKind::Numbered,
            BlockKind::Task,
            BlockKind::Quote,
            BlockKind::Code,
            BlockKind::Rule,
            BlockKind::TableRow,
            BlockKind::TableRow,
            BlockKind::Paragraph,
            BlockKind::Paragraph,
        ]
    );
    assert_eq!(b[0].level, 1);
    let p = &b[1].spans;
    assert!(p.iter().any(|s| s.bold && s.text == "bold"));
    assert!(p.iter().any(|s| s.italic && s.text == "it"));
    assert!(p.iter().any(|s| s.code && s.text == "code"));
    assert!(p
        .iter()
        .any(|s| s.link.as_deref() == Some("https://x.y") && s.text == "a link"));
    assert_eq!(b[4].level, 2);
    assert_eq!(b[4].number, 1);
    assert!(b[5].flag);
    assert_eq!(b[7].spans[0].text, "fn main() {}");
    assert!(b[9].flag, "header row");
    let row: String = b[10].spans.iter().map(|s| s.text.as_str()).collect();
    assert_eq!(row, "1  │  2");
    let pic: String = b[11].spans.iter().map(|s| s.text.as_str()).collect();
    assert_eq!(pic, "[picture: cat]");
    let raw: String = b[12].spans.iter().map(|s| s.text.as_str()).collect();
    assert_eq!(raw, "<b>raw</b>", "HTML is shown, never interpreted");
}

#[test]
fn viewer_through_a_vault_and_wiped_on_lock() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    std::fs::create_dir(&src).unwrap();
    std::fs::write(src.join("pic.png"), png(64, 32)).unwrap();
    std::fs::write(src.join("doc.pdf"), pdf()).unwrap();
    std::fs::write(src.join("notes.md"), "# Hi\n\ntext").unwrap();
    std::fs::write(src.join("blob.bin"), [0u8, 1, 2, 0, 3]).unwrap();
    let (mut v, _) = Vault::create(
        &tmp.path().join("v"),
        b"violet-harbor-tundra-9",
        Argon2Cost::MINIMUM,
    )
    .unwrap();
    let names = ["pic.png", "doc.pdf", "notes.md", "blob.bin"];
    v.import(&names.map(|n| src.join(n)), None, false, &mut |_| {})
        .unwrap();
    let s = crate::api::vault::strong_session(v, None);
    let id = |name: &str| {
        s.list(None, None)
            .unwrap()
            .into_iter()
            .find(|e| e.name == name)
            .unwrap()
            .id
    };

    let pic = s.open_viewer(id("pic.png")).unwrap();
    assert_eq!(pic.kind(), PreviewKind::Image);
    let r = pic.render_image(32, 32).unwrap();
    assert_eq!((r.width, r.height), (32, 16));

    let doc = s.open_viewer(id("doc.pdf")).unwrap();
    assert_eq!(doc.kind(), PreviewKind::Pdf);
    assert_eq!(doc.pdf_pages().unwrap().len(), 1);

    let md = s.open_viewer(id("notes.md")).unwrap();
    assert_eq!(md.kind(), PreviewKind::Markdown);
    assert_eq!(md.markdown().unwrap().len(), 2);
    assert_eq!(md.text().unwrap().lines, vec!["# Hi", "", "text"]);
    md.close();
    assert!(md.text().is_err());

    assert!(s.open_viewer(id("blob.bin")).is_err());

    s.lock();
    assert!(
        pic.render_image(10, 10).is_err(),
        "locking wipes open viewers"
    );
    assert!(doc.pdf_pages().is_err());
    assert!(s.viewers.lock().unwrap().is_empty());
}

fn gif(frames: u8) -> Vec<u8> {
    use image::codecs::gif::{GifEncoder, Repeat};
    let mut out = Vec::new();
    {
        let mut enc = GifEncoder::new(&mut out);
        enc.set_repeat(Repeat::Infinite).unwrap();
        for i in 0..frames {
            let img = RgbaImage::from_pixel(8, 4, image::Rgba([i * 80, 0, 0, 255]));
            let frame = image::Frame::from_parts(
                img,
                0,
                0,
                image::Delay::from_numer_denom_ms(if i == 0 { 10 } else { 250 }, 1),
            );
            enc.encode_frame(frame).unwrap();
        }
    }
    out
}

#[test]
fn animated_gifs_keep_every_frame() {
    let d = decode_image(&gif(3)).unwrap();
    assert_eq!(d.delays().len(), 3);
    assert_eq!(
        d.delays(),
        &[100, 250, 250],
        "too-short delays become 100 ms"
    );
    let f = d.render_frame(2, 100, 100).unwrap();
    assert_eq!((f.width, f.height), (8, 4));
    assert_eq!(&f.rgba[..4], &[160, 0, 0, 255]);
    assert!(d.render_frame(3, 10, 10).is_none());

    let still = decode_image(&gif(1)).unwrap();
    assert_eq!(still.delays().len(), 1);
    assert_eq!(decode_image(&png(4, 4)).unwrap().delays(), &[0]);
}

fn wav(rate: u32, seconds: u32) -> Vec<u8> {
    let frames = rate * seconds;
    let data_len = frames * 4;
    let mut w = Vec::new();
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + data_len).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes()); // PCM
    w.extend_from_slice(&2u16.to_le_bytes()); // stereo
    w.extend_from_slice(&rate.to_le_bytes());
    w.extend_from_slice(&(rate * 4).to_le_bytes());
    w.extend_from_slice(&4u16.to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&data_len.to_le_bytes());
    for i in 0..frames {
        let v = ((i as f32 * 440.0 * std::f32::consts::TAU / rate as f32).sin() * 8000.0) as i16;
        w.extend_from_slice(&v.to_le_bytes());
        w.extend_from_slice(&v.to_le_bytes());
    }
    w
}

#[test]
fn audio_files_are_recognised_and_read_in_memory() {
    let data = wav(8000, 2);
    // Recognised from its bytes even with the wrong name.
    assert_eq!(sniff("notes.bin", &data), Preview::Audio);
    assert_eq!(preview_for_name("song.flac"), Preview::Audio);
    let (kind, content) = Content::open_as("a.wav", Zeroizing::new(data), None).unwrap();
    assert_eq!(kind, Preview::Audio);
    let Content::Audio(a) = content else { panic!() };
    assert_eq!(a.info().duration_ms, 2000);
    assert_eq!(a.info().sample_rate, 8000);
    assert_eq!(a.info().channels, 2);
    // Not played yet: nothing touched the sound device.
    assert!(!a.state().playing);
    assert!(Content::open_as("b.mp3", Zeroizing::new(b"ID3 junk".to_vec()), None).is_err());
}

#[test]
fn open_as_overrides_detection() {
    let data = b"ssh-ed25519 AAAA key".to_vec();
    // Plain text is shown as text anyway; a forced kind wins.
    let (k, _) = Content::open_as("id.pub", Zeroizing::new(data.clone()), None).unwrap();
    assert_eq!(k, Preview::Text);
    let (k, _) = Content::open_as(
        "id.pub",
        Zeroizing::new(data.clone()),
        Some(Preview::Markdown),
    )
    .unwrap();
    assert_eq!(k, Preview::Markdown);
    // Binary forced to text still opens (lossy), but is not editable.
    let (k, c) = Content::open_as(
        "x.bin",
        Zeroizing::new(vec![0xff, 0, 1, 2]),
        Some(Preview::Text),
    )
    .unwrap();
    assert_eq!(k, Preview::Text);
    assert!(matches!(
        c,
        Content::Text {
            editable: false,
            ..
        }
    ));
    // Forced to a picture that is not one: a clear error.
    assert!(Content::open_as("x.bin", Zeroizing::new(data), Some(Preview::Image)).is_err());
}

#[test]
fn pdf_text_can_be_searched() {
    let doc = PdfDoc::open(Zeroizing::new(pdf_with(
        b"BT /F1 12 Tf 10 70 Td (Meeting at noon) Tj 0 -20 Td (Bring the Atropos keys) Tj ET",
    )))
    .unwrap();
    let text = doc.page_text(0);
    assert!(text.contains("Meeting at noon"), "{text:?}");
    assert!(text.contains("\n"), "lines split: {text:?}");
    let hits = doc.search("atropos", false);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].page, 0);
    assert!(hits[0].snippet.contains("Atropos"));
    assert!(doc.search("atropos", true).is_empty());
    assert!(doc.search("  ", false).is_empty());
}

#[test]
fn picture_edits_rotate_crop_and_save_in_the_same_format() {
    use super::edit::{apply, encode, ImageOp};
    let mut img = RgbaImage::new(40, 20);
    img.put_pixel(0, 0, image::Rgba([255, 0, 0, 255]));
    let r = apply(&img, &[ImageOp::RotateRight]).unwrap();
    assert_eq!((r.width(), r.height()), (20, 40));
    // The top-left corner moves to the top-right after a right turn.
    assert_eq!(r.get_pixel(19, 0).0, [255, 0, 0, 255]);
    let back = apply(&img, &[ImageOp::RotateRight, ImageOp::RotateLeft]).unwrap();
    assert_eq!(back, img);
    let f = apply(&img, &[ImageOp::FlipHorizontal]).unwrap();
    assert_eq!(f.get_pixel(39, 0).0, [255, 0, 0, 255]);
    let c = apply(
        &img,
        &[ImageOp::Crop {
            left: 0.5,
            top: 0.0,
            right: 1.0,
            bottom: 0.5,
        }],
    )
    .unwrap();
    assert_eq!((c.width(), c.height()), (20, 10));
    assert!(apply(
        &img,
        &[ImageOp::Crop {
            left: 0.6,
            top: 0.0,
            right: 0.4,
            bottom: 1.0,
        }]
    )
    .is_err());
    let s = apply(&img, &[ImageOp::Resize { max_side: 10 }]).unwrap();
    assert_eq!((s.width(), s.height()), (10, 5));
    let png = encode(c.clone(), "shot.png").unwrap();
    assert_eq!(&png[..4], b"\x89PNG");
    let jpg = encode(c, "shot.JPG").unwrap();
    assert_eq!(&jpg[..2], b"\xff\xd8");
    // Plain pictures have no camera details.
    let d = decode_image(&png).unwrap();
    assert!(d.details().is_empty());
}
