//! Thin, safe-ish wrapper over macOS PDFKit: open a PDF, render pages to pixels,
//! pick text between two points, read metadata, import existing markup
//! annotations and write our annotations back into the file.
//!
//! All coordinates handed to the rest of the app are "view points": the page as
//! it is shown (rotation applied), origin top-left, 1 unit = 1 PDF point.

use std::ffi::c_void;
use std::path::Path;

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::AnyThread;
use objc2_app_kit::{NSColor, NSColorSpace};
use objc2_core_foundation::{CGAffineTransform, CGPoint, CGRect, CGSize};
use objc2_core_graphics::{CGColorSpace, CGContext};
use objc2_foundation::{NSString, NSURL};
use objc2_pdf_kit::{
    PDFAnnotation, PDFAnnotationSubtypeHighlight, PDFAnnotationSubtypeStrikeOut,
    PDFAnnotationSubtypeText, PDFAnnotationSubtypeUnderline, PDFDisplayBox, PDFDocument,
    PDFDocumentAuthorAttribute, PDFDocumentCreationDateAttribute, PDFDocumentTitleAttribute, PDFPage,
};

use super::{clean_text, merge_adjacent_lines, nfc, RenderedPage};
use crate::model::{Annotation, Kind, ViewRect};

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGBitmapContextCreate(
        data: *mut c_void,
        width: usize,
        height: usize,
        bits_per_component: usize,
        bytes_per_row: usize,
        space: *const CGColorSpace,
        bitmap_info: u32,
    ) -> *mut CGContext;
    fn CGColorSpaceCreateDeviceRGB() -> *mut CGColorSpace;
    fn CGColorSpaceRelease(space: *mut CGColorSpace);
    fn CGContextRelease(c: *mut CGContext);
    fn CGContextSetRGBFillColor(c: *mut CGContext, r: f64, g: f64, b: f64, a: f64);
    fn CGContextFillRect(c: *mut CGContext, rect: CGRect);
    fn CGContextScaleCTM(c: *mut CGContext, sx: f64, sy: f64);
}

const K_CG_IMAGE_ALPHA_PREMULTIPLIED_LAST: u32 = 1;
const K_CG_BITMAP_BYTE_ORDER_32_BIG: u32 = 4 << 12;

pub struct PdfDoc {
    doc: Retained<PDFDocument>,
    pages: Vec<Retained<PDFPage>>,
    /// Printed page labels, read once (asking PDFKit every frame is slow).
    labels: std::cell::RefCell<Vec<Option<String>>>,
}

fn affine_apply(t: &CGAffineTransform, x: f64, y: f64) -> (f64, f64) {
    (t.a * x + t.c * y + t.tx, t.b * x + t.d * y + t.ty)
}

fn affine_invert(t: &CGAffineTransform) -> CGAffineTransform {
    let det = t.a * t.d - t.b * t.c;
    if det.abs() < 1e-12 {
        return *t;
    }
    let a = t.d / det;
    let b = -t.b / det;
    let c = -t.c / det;
    let d = t.a / det;
    let tx = -(a * t.tx + c * t.ty);
    let ty = -(b * t.tx + d * t.ty);
    CGAffineTransform { a, b, c, d, tx, ty }
}

fn ns(s: &str) -> Retained<NSString> {
    NSString::from_str(s)
}

impl PdfDoc {
    pub fn open(path: &Path) -> Option<PdfDoc> {
        unsafe {
            let url = NSURL::fileURLWithPath(&ns(&path.to_string_lossy()));
            let doc = PDFDocument::initWithURL(PDFDocument::alloc(), &url)?;
            let count = doc.pageCount();
            let mut pages = Vec::with_capacity(count);
            for i in 0..count {
                pages.push(doc.pageAtIndex(i)?);
            }
            let n = pages.len();
            Some(PdfDoc { doc, pages, labels: std::cell::RefCell::new(vec![None; n]) })
        }
    }

    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    /// Page label as printed (e.g. "iv" or "12"), falling back to the number.
    pub fn page_label(&self, index: usize) -> String {
        if let Some(Some(l)) = self.labels.borrow().get(index) {
            return l.clone();
        }
        let l = unsafe {
            self.pages
                .get(index)
                .and_then(|p| p.label())
                .map(|l| nfc(&l.to_string()))
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| (index + 1).to_string())
        };
        if let Some(slot) = self.labels.borrow_mut().get_mut(index) {
            *slot = Some(l.clone());
        }
        l
    }

    fn box_rect(page: &PDFPage) -> CGRect {
        unsafe { page.boundsForBox(PDFDisplayBox::CropBox) }
    }

    /// Size of the page as displayed (rotation applied), in points.
    pub fn page_size(&self, index: usize) -> (f64, f64) {
        let Some(page) = self.pages.get(index) else { return (612.0, 792.0) };
        unsafe {
            let r = Self::box_rect(page);
            let rot = page.rotation().rem_euclid(360);
            if rot == 90 || rot == 270 {
                (r.size.height, r.size.width)
            } else {
                (r.size.width, r.size.height)
            }
        }
    }

    fn transform(&self, page: &PDFPage) -> CGAffineTransform {
        unsafe { page.transformForBox(PDFDisplayBox::CropBox) }
    }

    /// View point (origin top-left) -> PDF page space.
    fn view_to_page(&self, index: usize, x: f64, y: f64) -> CGPoint {
        let page = &self.pages[index];
        let (_, h) = self.page_size(index);
        let inv = affine_invert(&self.transform(page));
        let (px, py) = affine_apply(&inv, x, h - y);
        CGPoint { x: px, y: py }
    }

    /// PDF page-space rectangle -> view rectangle (origin top-left).
    fn page_rect_to_view(&self, index: usize, r: CGRect) -> ViewRect {
        let page = &self.pages[index];
        let (_, h) = self.page_size(index);
        let t = self.transform(page);
        let corners = [
            (r.origin.x, r.origin.y),
            (r.origin.x + r.size.width, r.origin.y),
            (r.origin.x, r.origin.y + r.size.height),
            (r.origin.x + r.size.width, r.origin.y + r.size.height),
        ];
        let mut min_x = f64::MAX;
        let mut min_y = f64::MAX;
        let mut max_x = f64::MIN;
        let mut max_y = f64::MIN;
        for (cx, cy) in corners {
            let (bx, by) = affine_apply(&t, cx, cy);
            let vy = h - by;
            min_x = min_x.min(bx);
            max_x = max_x.max(bx);
            min_y = min_y.min(vy);
            max_y = max_y.max(vy);
        }
        ViewRect { x: min_x as f32, y: min_y as f32, w: (max_x - min_x) as f32, h: (max_y - min_y) as f32 }
    }

    fn view_rect_to_page(&self, index: usize, r: &ViewRect) -> CGRect {
        let a = self.view_to_page(index, r.x as f64, r.y as f64);
        let b = self.view_to_page(index, (r.x + r.w) as f64, (r.y + r.h) as f64);
        let x0 = a.x.min(b.x);
        let y0 = a.y.min(b.y);
        CGRect {
            origin: CGPoint { x: x0, y: y0 },
            size: CGSize { width: (a.x - b.x).abs(), height: (a.y - b.y).abs() },
        }
    }

    /// Render one page at `scale` pixels per point, white background.
    pub fn render(&self, index: usize, scale: f64) -> Option<RenderedPage> {
        let page = self.pages.get(index)?;
        let (w, h) = self.page_size(index);
        let width = ((w * scale).round() as usize).max(1);
        let height = ((h * scale).round() as usize).max(1);
        let stride = width * 4;
        let mut rgba = vec![0u8; stride * height];
        unsafe {
            let space = CGColorSpaceCreateDeviceRGB();
            let ctx = CGBitmapContextCreate(
                rgba.as_mut_ptr() as *mut c_void,
                width,
                height,
                8,
                stride,
                space,
                K_CG_IMAGE_ALPHA_PREMULTIPLIED_LAST | K_CG_BITMAP_BYTE_ORDER_32_BIG,
            );
            CGColorSpaceRelease(space);
            if ctx.is_null() {
                return None;
            }
            CGContextSetRGBFillColor(ctx, 1.0, 1.0, 1.0, 1.0);
            CGContextFillRect(
                ctx,
                CGRect { origin: CGPoint { x: 0.0, y: 0.0 }, size: CGSize { width: width as f64, height: height as f64 } },
            );
            CGContextScaleCTM(ctx, scale, scale);
            page.drawWithBox_toContext(PDFDisplayBox::CropBox, &*ctx);
            CGContextRelease(ctx);
        }
        Some(RenderedPage { width, height, rgba })
    }

    /// Text selected by dragging from `a` to `b` (view points) and the line
    /// rectangles it covers.
    pub fn select_text(&self, index: usize, a: (f32, f32), b: (f32, f32)) -> Option<(String, Vec<ViewRect>)> {
        let page = self.pages.get(index)?;
        unsafe {
            let pa = self.view_to_page(index, a.0 as f64, a.1 as f64);
            let pb = self.view_to_page(index, b.0 as f64, b.1 as f64);
            // Whole words, like any Mac app: start at the beginning of the
            // first word and end at the end of the last (either direction).
            let (ia, ib) = (page.characterIndexAtPoint(pa), page.characterIndexAtPoint(pb));
            let (s0, e0) = if ia >= 0 && ib >= 0 && ia > ib { (pb, pa) } else { (pa, pb) };
            let word_edge = |p: CGPoint, end: bool| -> CGPoint {
                page.selectionForWordAtPoint(p)
                    .map(|w| w.boundsForPage(page))
                    .filter(|r| r.size.width > 0.5 && r.size.height > 0.5)
                    .map(|r| CGPoint {
                        x: if end { r.origin.x + r.size.width - 0.3 } else { r.origin.x + 0.3 },
                        y: r.origin.y + r.size.height / 2.0,
                    })
                    .unwrap_or(p)
            };
            let (sa, sb) = (word_edge(s0, false), word_edge(e0, true));
            let sel = page.selectionFromPoint_toPoint(sa, sb).or_else(|| page.selectionFromPoint_toPoint(pa, pb))?;
            let text = sel.string().map(|s| s.to_string()).unwrap_or_default();
            let text = clean_text(&text);
            if text.is_empty() {
                return None;
            }
            let mut rects = Vec::new();
            for line in sel.selectionsByLine().iter() {
                let r = line.boundsForPage(page);
                if r.size.width > 0.5 && r.size.height > 0.5 {
                    rects.push(self.page_rect_to_view(index, r));
                }
            }
            if rects.is_empty() {
                rects.push(self.page_rect_to_view(index, sel.boundsForPage(page)));
            }
            Some((text, rects))
        }
    }

    /// Text inside a rectangle (used for image selections, as alt text).
    pub fn text_in_rect(&self, index: usize, r: &ViewRect) -> String {
        let Some(page) = self.pages.get(index) else { return String::new() };
        unsafe {
            let pr = self.view_rect_to_page(index, r);
            page.selectionForRect(pr)
                .and_then(|s| s.string())
                .map(|s| clean_text(&s.to_string()))
                .unwrap_or_default()
        }
    }

    /// The word under a click (view points).
    pub fn word_at(&self, index: usize, p: (f32, f32)) -> Option<(String, Vec<ViewRect>)> {
        let page = self.pages.get(index)?;
        unsafe {
            let pt = self.view_to_page(index, p.0 as f64, p.1 as f64);
            let sel = page.selectionForWordAtPoint(pt)?;
            let text = clean_text(&sel.string()?.to_string());
            if text.is_empty() {
                return None;
            }
            let r = sel.boundsForPage(page);
            if r.size.width < 0.5 {
                return None;
            }
            Some((text, vec![self.page_rect_to_view(index, r)]))
        }
    }

    /// Year the file says it was created, if any.
    pub fn creation_year(&self) -> Option<i32> {
        unsafe {
            let attrs = self.doc.documentAttributes()?;
            let obj: Retained<AnyObject> = attrs.objectForKey(PDFDocumentCreationDateAttribute)?;
            let d = obj.downcast::<objc2_foundation::NSDate>().ok()?;
            let secs = d.timeIntervalSince1970();
            chrono::DateTime::from_timestamp(secs as i64, 0).map(|t| chrono::Datelike::year(&t))
        }
    }

    pub fn page_text(&self, index: usize) -> String {
        unsafe { self.pages.get(index).and_then(|p| p.string()).map(|s| nfc(&s.to_string())).unwrap_or_default() }
    }

    fn attribute(&self, key: &NSString) -> Option<String> {
        unsafe {
            let attrs = self.doc.documentAttributes()?;
            let obj: Retained<AnyObject> = attrs.objectForKey(key)?;
            let s = obj.downcast::<NSString>().ok()?;
            let s = nfc(&s.to_string());
            if s.trim().is_empty() { None } else { Some(s) }
        }
    }

    pub fn title(&self) -> Option<String> {
        unsafe { self.attribute(PDFDocumentTitleAttribute) }
    }

    pub fn author(&self) -> Option<String> {
        unsafe { self.attribute(PDFDocumentAuthorAttribute) }
    }

    /// Take the markup annotations (highlight, underline, strike-out, notes)
    /// already in the file into our model, and remove them from the in-memory
    /// document so they are drawn only once (by us).
    pub fn take_existing_annotations(&self) -> Vec<Annotation> {
        let mut out = Vec::new();
        for (index, page) in self.pages.iter().enumerate() {
            unsafe {
                let annots: Vec<Retained<PDFAnnotation>> = page.annotations().iter().collect();
                for an in annots {
                    let Some(kind_str) = an.r#type().map(|t| t.to_string()) else { continue };
                    let kind = match kind_str.as_str() {
                        "Highlight" => Kind::Highlight,
                        "Underline" => Kind::Underline,
                        "StrikeOut" => Kind::Strike,
                        "Text" => Kind::Note,
                        _ => continue,
                    };
                    let bounds = an.bounds();
                    let view = self.page_rect_to_view(index, bounds);
                    let color = color_index_of(&an.color());
                    let text = if kind == Kind::Note {
                        String::new()
                    } else {
                        let grown = CGRect {
                            origin: CGPoint { x: bounds.origin.x, y: bounds.origin.y - 2.0 },
                            size: CGSize { width: bounds.size.width, height: bounds.size.height.max(10.0) + 4.0 },
                        };
                        [bounds, grown]
                            .iter()
                            .filter_map(|r| page.selectionForRect(*r).and_then(|s| s.string()))
                            .map(|s| clean_text(&s.to_string()))
                            .find(|t| !t.is_empty())
                            .unwrap_or_default()
                    };
                    let comment = an.contents().map(|c| c.to_string()).unwrap_or_default();
                    let author = an.userName().map(|c| c.to_string()).unwrap_or_default();
                    let mut a = Annotation::new(kind, index, color, vec![view], text);
                    a.comment = comment;
                    a.author = author;
                    out.push(a);
                    page.removeAnnotation(&an);
                }
            }
        }
        merge_adjacent_lines(out)
    }

    /// Write `annots` into the PDF file at `path` (one markup annotation per
    /// line rectangle, comment on the first). Image selections stay outside.
    pub fn save_with_annotations(&self, path: &Path, annots: &[Annotation], palette: &[[u8; 3]]) -> bool {
        let mut added: Vec<(usize, Retained<PDFAnnotation>)> = Vec::new();
        unsafe {
            for a in annots {
                let Some(page) = self.pages.get(a.page) else { continue };
                let subtype: &NSString = match a.kind {
                    Kind::Highlight => PDFAnnotationSubtypeHighlight,
                    Kind::Underline => PDFAnnotationSubtypeUnderline,
                    Kind::Strike => PDFAnnotationSubtypeStrikeOut,
                    Kind::Note => PDFAnnotationSubtypeText,
                    Kind::Image => continue,
                };
                let c = palette.get(a.color).copied().unwrap_or([255, 230, 0]);
                let color = NSColor::colorWithSRGBRed_green_blue_alpha(
                    c[0] as f64 / 255.0,
                    c[1] as f64 / 255.0,
                    c[2] as f64 / 255.0,
                    1.0,
                );
                for (i, r) in a.rects.iter().enumerate() {
                    let mut pr = self.view_rect_to_page(a.page, r);
                    if a.kind == Kind::Note {
                        pr.size = CGSize { width: 20.0, height: 20.0 };
                    }
                    let an = PDFAnnotation::initWithBounds_forType_withProperties(
                        PDFAnnotation::alloc(),
                        pr,
                        subtype,
                        None,
                    );
                    an.setColor(&color);
                    if i == 0 && !a.comment.is_empty() {
                        an.setContents(Some(&ns(&a.comment)));
                    }
                    if !a.author.is_empty() {
                        an.setUserName(Some(&ns(&a.author)));
                    }
                    page.addAnnotation(&an);
                    added.push((a.page, an));
                }
            }
            let ok = self.doc.writeToURL(&NSURL::fileURLWithPath(&ns(&path.to_string_lossy())));
            // Keep the in-memory document free of them: we draw our own.
            for (p, an) in added {
                self.pages[p].removeAnnotation(&an);
            }
            ok
        }
    }
}

fn color_index_of(color: &NSColor) -> usize {
    {
        let Some(c) = color.colorUsingColorSpace(&NSColorSpace::sRGBColorSpace()) else { return 2 };
        let rgb = [
            (c.redComponent() * 255.0) as i32,
            (c.greenComponent() * 255.0) as i32,
            (c.blueComponent() * 255.0) as i32,
        ];
        crate::model::nearest_color(rgb)
    }
}
