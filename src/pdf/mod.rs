//! Reading and writing PDFs: open a file, render pages to pixels, pick text
//! between two points, read metadata, import existing markup annotations and
//! write our annotations back into the file.
//!
//! Two engines sit behind the same `PdfDoc`:
//! - macOS: Apple's PDFKit (`mac.rs`) — nothing extra to ship.
//! - Windows / Linux: PDFium (`pdfium.rs`) — needs the pdfium library file
//!   next to the program. On a Mac, `--features pdfium` picks it too (for testing).
//!
//! All coordinates handed to the rest of the app are "view points": the page as
//! it is shown (rotation applied), origin top-left, 1 unit = 1 PDF point.

use crate::model::{Annotation, Kind};

#[cfg(all(target_os = "macos", not(feature = "pdfium")))]
mod mac;
#[cfg(all(target_os = "macos", not(feature = "pdfium")))]
pub use mac::PdfDoc;

#[cfg(any(not(target_os = "macos"), feature = "pdfium"))]
mod pdfium;
#[cfg(any(not(target_os = "macos"), feature = "pdfium"))]
pub use pdfium::PdfDoc;

/// Why no PDF can be opened at all, if that is the case (the PDFium library
/// file is missing next to the program). `None` on a Mac: PDFKit is always there.
pub fn engine_problem() -> Option<String> {
    #[cfg(any(not(target_os = "macos"), feature = "pdfium"))]
    if pdfium::engine_missing() {
        let file = if cfg!(windows) { "pdfium.dll" } else if cfg!(target_os = "macos") { "libpdfium.dylib" } else { "libpdfium.so" };
        return Some(format!("PDF 엔진 파일({file})을 찾지 못했어요. 프로그램과 같은 폴더에 있어야 해요."));
    }
    None
}

pub struct RenderedPage {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

/// Join PDF annotations that were written one-per-line back into one note.
pub(crate) fn merge_adjacent_lines(list: Vec<Annotation>) -> Vec<Annotation> {
    let mut out: Vec<Annotation> = Vec::new();
    for a in list {
        if let Some(last) = out.last_mut() {
            let lr = last.rects.last().copied();
            if let (Some(lr), Some(r)) = (lr, a.rects.first()) {
                let next_line = r.y >= lr.y + lr.h * 0.5 && r.y <= lr.y + lr.h * 2.2;
                if last.kind == a.kind
                    && last.page == a.page
                    && last.color == a.color
                    && a.comment.is_empty()
                    && a.kind != Kind::Note
                    && next_line
                {
                    last.rects.extend(a.rects.iter().copied());
                    if !a.text.is_empty() {
                        if last.text.ends_with('-') {
                            last.text.pop();
                            last.text.push_str(&a.text);
                        } else {
                            last.text.push(' ');
                            last.text.push_str(&a.text);
                        }
                    }
                    continue;
                }
            }
        }
        out.push(a);
    }
    out
}

/// Korean text in composed form: macOS file names and some PDFs keep Hangul
/// as separate letters (ㅎ ㅏ ㄴ), which look broken on screen and in notes.
pub fn nfc(s: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    s.nfc().collect()
}

/// Join hyphenated line breaks and collapse whitespace.
pub fn clean_text(s: &str) -> String {
    let s = &nfc(s);
    let mut out = String::with_capacity(s.len());
    let lines: Vec<&str> = s.lines().map(|l| l.trim()).filter(|l| !l.is_empty()).collect();
    for (i, line) in lines.iter().enumerate() {
        if i > 0 {
            if out.ends_with('-') && line.chars().next().map(|c| c.is_lowercase()).unwrap_or(false) {
                out.pop();
            } else {
                out.push(' ');
            }
        }
        out.push_str(line);
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Dark version of a rendered page: lightness turned around, colours kept
/// (white paper -> near-black, black text -> soft white, red stays red).
pub fn darken(img: &mut RenderedPage) {
    const LO: f32 = 26.0;
    const HI: f32 = 222.0;
    for px in img.rgba.chunks_exact_mut(4) {
        let (r, g, b) = (px[0] as f32, px[1] as f32, px[2] as f32);
        let y = 0.299 * r + 0.587 * g + 0.114 * b;
        let ny = LO + (255.0 - y) * (HI - LO) / 255.0;
        let d = ny - y;
        px[0] = (r + d).clamp(0.0, 255.0) as u8;
        px[1] = (g + d).clamp(0.0, 255.0) as u8;
        px[2] = (b + d).clamp(0.0, 255.0) as u8;
    }
}
