//! The PDF engine for Windows and Linux: Google's PDFium (the one inside
//! Chrome), loaded from the `pdfium` library file that ships next to the
//! program. Same `PdfDoc` as the Mac's PDFKit version in `mac.rs`.
//!
//! PDFium gives us letters and their boxes; picking words and lines out of
//! them (what PDFKit does for us on a Mac) is done here.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::OnceLock;

use pdfium_render::prelude::*;

use super::{clean_text, merge_adjacent_lines, nfc, RenderedPage};
use crate::model::{Annotation, Kind, ViewRect};

static ENGINE: OnceLock<Option<Pdfium>> = OnceLock::new();

/// Folders where the pdfium library may sit: `PDFIUM_DIR`, next to the
/// program (Windows/Linux), inside the app bundle (Mac), the current folder.
fn library_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(d) = std::env::var_os("PDFIUM_DIR") {
        dirs.push(d.into());
    }
    if let Some(d) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
        dirs.push(d.clone());
        dirs.push(d.join("lib"));
        dirs.push(d.join("../Frameworks"));
    }
    if let Ok(d) = std::env::current_dir() {
        dirs.push(d.clone());
        dirs.push(d.join("pdfium"));
    }
    dirs
}

fn engine() -> Option<&'static Pdfium> {
    ENGINE
        .get_or_init(|| {
            for d in library_dirs() {
                let file = Pdfium::pdfium_platform_library_name_at_path(&d);
                if file.is_file() {
                    match Pdfium::bind_to_library(&file) {
                        Ok(b) => return Some(Pdfium::new(b)),
                        Err(e) => eprintln!("could not load {}: {e:?}", file.display()),
                    }
                }
            }
            match Pdfium::bind_to_system_library() {
                Ok(b) => Some(Pdfium::new(b)),
                Err(e) => {
                    eprintln!("PDF engine (pdfium) not found: {e:?}");
                    None
                }
            }
        })
        .as_ref()
}

/// Is the PDF engine there? (When not, no PDF can be opened: say why.)
pub fn engine_missing() -> bool {
    engine().is_none()
}

/// Page space -> view space: `vx = a·x + c·y + tx`, `vy = b·x + d·y + ty`.
#[derive(Clone, Copy)]
struct Affine {
    a: f64,
    b: f64,
    c: f64,
    d: f64,
    tx: f64,
    ty: f64,
}

impl Affine {
    fn apply(&self, x: f64, y: f64) -> (f64, f64) {
        (self.a * x + self.c * y + self.tx, self.b * x + self.d * y + self.ty)
    }

    fn invert(&self) -> Affine {
        let det = self.a * self.d - self.b * self.c;
        if det.abs() < 1e-12 {
            return *self;
        }
        let a = self.d / det;
        let b = -self.b / det;
        let c = -self.c / det;
        let d = self.a / det;
        Affine { a, b, c, d, tx: -(a * self.tx + c * self.ty), ty: -(b * self.tx + d * self.ty) }
    }
}

/// One letter of a page, where it is shown.
#[derive(Clone, Copy)]
struct Ch {
    c: char,
    r: ViewRect,
    /// Has a real box (spaces and line ends PDFium makes up have none).
    solid: bool,
}

impl Ch {
    fn center(&self) -> (f32, f32) {
        (self.r.x + self.r.w / 2.0, self.r.y + self.r.h / 2.0)
    }

    fn is_word(&self) -> bool {
        self.solid && (self.c.is_alphanumeric() || matches!(self.c, '\'' | '’' | '_'))
    }
}

pub struct PdfDoc {
    // Pages first: they must be dropped before the document.
    pages: RefCell<Vec<PdfPage<'static>>>,
    doc: PdfDocument<'static>,
    sizes: Vec<(f64, f64)>,
    transforms: Vec<Affine>,
    chars: RefCell<HashMap<usize, Rc<Vec<Ch>>>>,
}

/// How the page's own coordinates land on the page as shown. PDFium knows
/// (crop box, rotation); we ask it about three points and keep the answer.
fn transform_of(page: &PdfPage) -> Affine {
    const S: f64 = 100.0;
    const FAR: f64 = 1000.0;
    let config = PdfRenderConfig::new().scale_page_by_factor(S as f32);
    let at = |x: f64, y: f64| -> Option<(f64, f64)> {
        page.points_to_pixels(PdfPoints::new(x as f32), PdfPoints::new(y as f32), &config).ok().map(|(px, py)| (px as f64 / S, py as f64 / S))
    };
    match (at(0.0, 0.0), at(FAR, 0.0), at(0.0, FAR)) {
        (Some(o), Some(x), Some(y)) => {
            // Pages turn in quarter steps: each of these is -1, 0 or 1.
            let unit = |v: f64| (v / FAR).round();
            Affine { a: unit(x.0 - o.0), b: unit(x.1 - o.1), c: unit(y.0 - o.0), d: unit(y.1 - o.1), tx: o.0, ty: o.1 }
        }
        _ => Affine { a: 1.0, b: 0.0, c: 0.0, d: -1.0, tx: 0.0, ty: page.height().value as f64 },
    }
}

impl PdfDoc {
    pub fn open(path: &Path) -> Option<PdfDoc> {
        let engine = engine()?;
        // Whole file in memory: the file itself stays free to be written over.
        let bytes = std::fs::read(path).ok()?;
        let doc = engine.load_pdf_from_byte_vec(bytes, None).ok()?;
        let count = doc.pages().len();
        let mut pages = Vec::with_capacity(count.max(0) as usize);
        for i in 0..count {
            pages.push(doc.pages().get(i).ok()?);
        }
        let sizes = pages.iter().map(|p| (p.width().value as f64, p.height().value as f64)).collect();
        let transforms = pages.iter().map(transform_of).collect();
        Some(PdfDoc { pages: RefCell::new(pages), doc, sizes, transforms, chars: RefCell::new(HashMap::new()) })
    }

    pub fn page_count(&self) -> usize {
        self.sizes.len()
    }

    /// Page label as printed (e.g. "iv" or "12"), falling back to the number.
    pub fn page_label(&self, index: usize) -> String {
        self.pages
            .borrow()
            .get(index)
            .and_then(|p| p.label().map(nfc))
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| (index + 1).to_string())
    }

    /// Size of the page as displayed (rotation applied), in points.
    pub fn page_size(&self, index: usize) -> (f64, f64) {
        self.sizes.get(index).copied().unwrap_or((612.0, 792.0))
    }

    /// PDF page-space rectangle -> view rectangle (origin top-left).
    fn page_rect_to_view(&self, index: usize, left: f64, bottom: f64, right: f64, top: f64) -> ViewRect {
        let t = self.transforms[index];
        let (ax, ay) = t.apply(left, bottom);
        let (bx, by) = t.apply(right, top);
        ViewRect { x: ax.min(bx) as f32, y: ay.min(by) as f32, w: (ax - bx).abs() as f32, h: (ay - by).abs() as f32 }
    }

    fn view_rect_to_page(&self, index: usize, r: &ViewRect) -> PdfRect {
        let inv = self.transforms[index].invert();
        let (ax, ay) = inv.apply(r.x as f64, r.y as f64);
        let (bx, by) = inv.apply((r.x + r.w) as f64, (r.y + r.h) as f64);
        PdfRect::new_from_values(ay.min(by) as f32, ax.min(bx) as f32, ay.max(by) as f32, ax.max(bx) as f32)
    }

    /// Render one page at `scale` pixels per point, white background.
    pub fn render(&self, index: usize, scale: f64) -> Option<RenderedPage> {
        let pages = self.pages.borrow();
        let page = pages.get(index)?;
        let (w, h) = self.page_size(index);
        let width = ((w * scale).round() as i32).max(1);
        let height = ((h * scale).round() as i32).max(1);
        let config = PdfRenderConfig::new().set_target_size(width, height).set_maximum_width(width).set_maximum_height(height);
        let bitmap = page.render_with_config(&config).ok()?;
        let (bw, bh) = (bitmap.width().max(0) as usize, bitmap.height().max(0) as usize);
        let rgba = bitmap.as_rgba_bytes();
        if bw == 0 || bh == 0 || rgba.len() < bw * bh * 4 {
            return None;
        }
        Some(RenderedPage { width: bw, height: bh, rgba })
    }

    /// Every letter of a page with its box, read once.
    fn chars(&self, index: usize) -> Rc<Vec<Ch>> {
        if let Some(c) = self.chars.borrow().get(&index) {
            return c.clone();
        }
        let mut out: Vec<Ch> = Vec::new();
        let pages = self.pages.borrow();
        if let Some(text) = pages.get(index).and_then(|p| p.text().ok()) {
            for ch in text.chars().iter() {
                let c = ch.unicode_char().unwrap_or(' ');
                let r = ch
                    .loose_bounds()
                    .ok()
                    .map(|b| self.page_rect_to_view(index, b.left().value as f64, b.bottom().value as f64, b.right().value as f64, b.top().value as f64))
                    .unwrap_or_default();
                let solid = r.w > 0.1 && r.h > 0.1 && !c.is_control();
                out.push(Ch { c, r, solid });
            }
        }
        let out = Rc::new(out);
        self.chars.borrow_mut().insert(index, out.clone());
        out
    }

    /// Text selected by dragging from `a` to `b` (view points) and the line
    /// rectangles it covers.
    pub fn select_text(&self, index: usize, a: (f32, f32), b: (f32, f32)) -> Option<(String, Vec<ViewRect>)> {
        if index >= self.page_count() {
            return None;
        }
        let chars = self.chars(index);
        let (ia, ib) = (nearest(&chars, a)?, nearest(&chars, b)?);
        // Whole words: from the start of the first to the end of the last.
        let (lo, hi) = (word_start(&chars, ia.min(ib)), word_end(&chars, ia.max(ib)));
        let text = clean_text(&string_of(&chars[lo..=hi]));
        if text.is_empty() {
            return None;
        }
        let rects = line_rects(&chars[lo..=hi]);
        if rects.is_empty() {
            return None;
        }
        Some((text, rects))
    }

    /// Text inside a rectangle (used for image selections, as alt text).
    pub fn text_in_rect(&self, index: usize, r: &ViewRect) -> String {
        if index >= self.page_count() {
            return String::new();
        }
        clean_text(&text_inside(&self.chars(index), r))
    }

    /// The word under a click (view points).
    pub fn word_at(&self, index: usize, p: (f32, f32)) -> Option<(String, Vec<ViewRect>)> {
        if index >= self.page_count() {
            return None;
        }
        let chars = self.chars(index);
        let i = chars.iter().position(|c| c.solid && contains(&c.r, p, 0.5))?;
        if !chars[i].is_word() {
            return None;
        }
        let (lo, hi) = (word_start(&chars, i), word_end(&chars, i));
        let text = clean_text(&string_of(&chars[lo..=hi]));
        let rects = line_rects(&chars[lo..=hi]);
        if text.is_empty() || rects.is_empty() {
            return None;
        }
        Some((text, rects))
    }

    fn meta(&self, tag: PdfDocumentMetadataTagType) -> Option<String> {
        let s = nfc(self.doc.metadata().get(tag)?.value());
        if s.trim().is_empty() { None } else { Some(s) }
    }

    /// Year the file says it was created, if any ("D:20210315…").
    pub fn creation_year(&self) -> Option<i32> {
        let d = self.meta(PdfDocumentMetadataTagType::CreationDate)?;
        let digits: String = d.trim().trim_start_matches("D:").chars().take(4).collect();
        digits.parse::<i32>().ok().filter(|y| (1900..2200).contains(y))
    }

    pub fn page_text(&self, index: usize) -> String {
        if index >= self.page_count() {
            return String::new();
        }
        nfc(&string_of(&self.chars(index)))
    }

    pub fn title(&self) -> Option<String> {
        self.meta(PdfDocumentMetadataTagType::Title)
    }

    pub fn author(&self) -> Option<String> {
        self.meta(PdfDocumentMetadataTagType::Author)
    }

    /// Take the markup annotations (highlight, underline, strike-out, notes)
    /// already in the file into our model, and remove them from the in-memory
    /// document so they are drawn only once (by us).
    pub fn take_existing_annotations(&self) -> Vec<Annotation> {
        let mut out = Vec::new();
        for index in 0..self.page_count() {
            // Read first (needs the page's letters), then remove.
            let mut found: Vec<(Annotation, PdfPageAnnotation<'static>)> = Vec::new();
            {
                let mut pages = self.pages.borrow_mut();
                let all = pages[index].annotations_mut();
                let list: Vec<PdfPageAnnotation<'static>> = (0..all.len()).filter_map(|i| all.get(i).ok()).collect();
                drop(pages);
                for an in list {
                    let kind = match &an {
                        PdfPageAnnotation::Highlight(_) => Kind::Highlight,
                        PdfPageAnnotation::Underline(_) => Kind::Underline,
                        PdfPageAnnotation::Strikeout(_) => Kind::Strike,
                        PdfPageAnnotation::Text(_) => Kind::Note,
                        _ => continue,
                    };
                    let Ok(bounds) = an.bounds() else { continue };
                    let whole = self.page_rect_to_view(index, bounds.left().value as f64, bounds.bottom().value as f64, bounds.right().value as f64, bounds.top().value as f64);
                    // One box per line when the file says where the lines are.
                    let mut rects: Vec<ViewRect> = if kind == Kind::Note {
                        Vec::new()
                    } else {
                        an.attachment_points()
                            .iter()
                            .map(|q| self.page_rect_to_view(index, q.left().value as f64, q.bottom().value as f64, q.right().value as f64, q.top().value as f64))
                            .filter(|r| r.w > 0.5 && r.h > 0.5)
                            .collect()
                    };
                    if rects.is_empty() {
                        rects.push(whole);
                    }
                    let text = if kind == Kind::Note {
                        String::new()
                    } else {
                        let chars = self.chars(index);
                        let parts: Vec<String> = rects
                            .iter()
                            .map(|r| {
                                let grown = ViewRect { x: r.x, y: r.y - 2.0, w: r.w, h: r.h.max(10.0) + 4.0 };
                                [*r, grown].iter().map(|r| clean_text(&text_inside(&chars, r))).find(|t| !t.is_empty()).unwrap_or_default()
                            })
                            .filter(|t| !t.is_empty())
                            .collect();
                        clean_text(&parts.join("\n"))
                    };
                    let color = an.stroke_color().map(|c| crate::model::nearest_color([c.red() as i32, c.green() as i32, c.blue() as i32])).unwrap_or(2);
                    let mut a = Annotation::new(kind, index, color, rects, text);
                    a.comment = an.contents().unwrap_or_default();
                    a.author = an.creator().unwrap_or_default();
                    found.push((a, an));
                }
            }
            let mut pages = self.pages.borrow_mut();
            for (a, an) in found {
                let _ = pages[index].annotations_mut().delete_annotation(an);
                out.push(a);
            }
        }
        merge_adjacent_lines(out)
    }

    /// Write `annots` into the PDF file at `path` (one markup annotation per
    /// line rectangle, comment on the first). Image selections stay outside.
    pub fn save_with_annotations(&self, path: &Path, annots: &[Annotation], palette: &[[u8; 3]]) -> bool {
        let mut added: Vec<(usize, PdfPageAnnotation<'static>)> = Vec::new();
        let mut pages = self.pages.borrow_mut();
        for a in annots {
            if a.kind == Kind::Image || a.page >= pages.len() {
                continue;
            }
            let c = palette.get(a.color).copied().unwrap_or([255, 230, 0]);
            let color = PdfColor::new(c[0], c[1], c[2], 255);
            for (i, r) in a.rects.iter().enumerate() {
                let mut pr = self.view_rect_to_page(a.page, r);
                if a.kind == Kind::Note {
                    pr = PdfRect::new_from_values(pr.bottom().value, pr.left().value, pr.bottom().value + 20.0, pr.left().value + 20.0);
                }
                let comment = if i == 0 { a.comment.as_str() } else { "" };
                let list = pages[a.page].annotations_mut();
                let made = (|| -> Result<PdfPageAnnotation<'static>, PdfiumError> {
                    Ok(match a.kind {
                        Kind::Highlight => {
                            let mut an = list.create_highlight_annotation()?;
                            an.attachment_points_mut().create_attachment_point_at_end(pr.to_quad_points())?;
                            PdfPageAnnotation::Highlight(an)
                        }
                        Kind::Underline => {
                            let mut an = list.create_underline_annotation()?;
                            an.attachment_points_mut().create_attachment_point_at_end(pr.to_quad_points())?;
                            PdfPageAnnotation::Underline(an)
                        }
                        Kind::Strike => {
                            let mut an = list.create_strikeout_annotation()?;
                            an.attachment_points_mut().create_attachment_point_at_end(pr.to_quad_points())?;
                            PdfPageAnnotation::Strikeout(an)
                        }
                        Kind::Note | Kind::Image => PdfPageAnnotation::Text(list.create_text_annotation(comment)?),
                    })
                })();
                let Ok(mut an) = made else { continue };
                let _ = an.set_bounds(pr);
                let _ = an.set_stroke_color(color);
                if !comment.is_empty() {
                    let _ = an.set_contents(comment);
                }
                if !a.author.is_empty() {
                    let _ = an.set_creator(&a.author);
                }
                added.push((a.page, an));
            }
        }
        let ok = match self.doc.save_to_bytes() {
            Ok(bytes) => write_safely(path, &bytes),
            Err(e) => {
                eprintln!("could not save the PDF: {e:?}");
                false
            }
        };
        // Keep the in-memory document free of them: we draw our own.
        for (p, an) in added {
            let _ = pages[p].annotations_mut().delete_annotation(an);
        }
        ok
    }
}

/// Write beside the file first, then swap: a failed write never leaves half a PDF.
fn write_safely(path: &Path, bytes: &[u8]) -> bool {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".saving");
    let tmp = PathBuf::from(tmp);
    if std::fs::write(&tmp, bytes).is_err() {
        let _ = std::fs::remove_file(&tmp);
        return false;
    }
    if std::fs::rename(&tmp, path).is_ok() {
        return true;
    }
    // Some folders (network drives) refuse the swap: write straight in.
    let ok = std::fs::write(path, bytes).is_ok();
    let _ = std::fs::remove_file(&tmp);
    ok
}

fn contains(r: &ViewRect, p: (f32, f32), slack: f32) -> bool {
    p.0 >= r.x - slack && p.0 <= r.x + r.w + slack && p.1 >= r.y - slack && p.1 <= r.y + r.h + slack
}

/// The letter under a point, or else the closest one (lines above and below
/// count as further away than letters along the same line).
fn nearest(chars: &[Ch], p: (f32, f32)) -> Option<usize> {
    let mut best: Option<(usize, f32)> = None;
    for (i, c) in chars.iter().enumerate() {
        if !c.solid {
            continue;
        }
        if contains(&c.r, p, 0.0) {
            return Some(i);
        }
        let dx = (c.r.x - p.0).max(p.0 - (c.r.x + c.r.w)).max(0.0);
        let dy = (c.r.y - p.1).max(p.1 - (c.r.y + c.r.h)).max(0.0);
        let d = dx + dy * 3.0;
        if best.is_none_or(|(_, bd)| d < bd) {
            best = Some((i, d));
        }
    }
    best.map(|(i, _)| i)
}

fn word_start(chars: &[Ch], mut i: usize) -> usize {
    while i > 0 && chars[i].is_word() && chars[i - 1].is_word() {
        i -= 1;
    }
    i
}

fn word_end(chars: &[Ch], mut i: usize) -> usize {
    while i + 1 < chars.len() && chars[i].is_word() && chars[i + 1].is_word() {
        i += 1;
    }
    i
}

/// The letters as text. PDFium marks a word broken across lines with U+0002.
fn string_of(chars: &[Ch]) -> String {
    let mut s = String::with_capacity(chars.len());
    for c in chars {
        match c.c {
            '\u{2}' => s.push_str("-\n"),
            '\r' | '\0' | '\u{fffe}' | '\u{ffff}' => {}
            c => s.push(c),
        }
    }
    s
}

/// Text of the letters whose middle is inside `r` (made-up spaces and line
/// ends follow the letter before them).
fn text_inside(chars: &[Ch], r: &ViewRect) -> String {
    let mut picked: Vec<Ch> = Vec::new();
    let mut last_in = false;
    for c in chars {
        if c.solid {
            last_in = contains(r, c.center(), 0.0);
        }
        if last_in {
            picked.push(*c);
        }
    }
    string_of(&picked)
}

/// One rectangle per line of the given letters.
fn line_rects(chars: &[Ch]) -> Vec<ViewRect> {
    let mut out: Vec<ViewRect> = Vec::new();
    for c in chars.iter().filter(|c| c.solid) {
        if let Some(cur) = out.last_mut() {
            let overlap = (cur.y + cur.h).min(c.r.y + c.r.h) - cur.y.max(c.r.y);
            let same_line = overlap > 0.5 * cur.h.min(c.r.h);
            // A jump far back to the left on the same height is another column.
            if same_line && c.r.x + c.r.w >= cur.x - 1.0 {
                let (x0, y0) = (cur.x.min(c.r.x), cur.y.min(c.r.y));
                let (x1, y1) = ((cur.x + cur.w).max(c.r.x + c.r.w), (cur.y + cur.h).max(c.r.y + c.r.h));
                *cur = ViewRect { x: x0, y: y0, w: x1 - x0, h: y1 - y0 };
                continue;
            }
        }
        out.push(c.r);
    }
    out.retain(|r| r.w > 0.5 && r.h > 0.5);
    out
}
