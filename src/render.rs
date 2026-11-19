//! Page pictures are drawn on a background thread with its own copy of the
//! PDF, so scrolling and zooming never wait for the PDF engine.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

use eframe::egui::{self, TextureHandle, TextureOptions};

use crate::pdf::{self, PdfDoc};

/// Zoom levels step by 1.25×; pages are drawn at the level nearest the zoom
/// and scaled the small rest of the way.
pub const LEVEL_STEP: f32 = 1.25;

pub fn level_for(pixels_per_point_scale: f32) -> u32 {
    ((pixels_per_point_scale.max(0.05).ln() / LEVEL_STEP.ln()).round() as i32 + 40).max(0) as u32
}

pub fn scale_of(level: u32) -> f64 {
    (LEVEL_STEP as f64).powi(level as i32 - 40)
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Key {
    pub page: usize,
    /// Zoom level, or `THUMB` for sidebar pictures.
    pub level: u32,
    pub dark: bool,
}

pub const THUMB: u32 = u32::MAX;

pub struct Done {
    pub key: Key,
    /// None when the job was skipped (zoom changed meanwhile).
    pub texture: Option<TextureHandle>,
}

pub struct Renderer {
    tx: Sender<(Key, f64)>,
    pub done: Receiver<Done>,
    wanted_level: Arc<AtomicU32>,
    requested: HashSet<Key>,
}

impl Renderer {
    pub fn start(path: PathBuf, ctx: egui::Context) -> Renderer {
        let (tx, rx) = channel::<(Key, f64)>();
        let (dtx, done) = channel::<Done>();
        let wanted_level = Arc::new(AtomicU32::new(40));
        let wanted = wanted_level.clone();
        std::thread::Builder::new()
            .name("page-renderer".into())
            .spawn(move || {
                let Some(doc) = PdfDoc::open(&path) else { return };
                // We draw annotations ourselves.
                let _ = doc.take_existing_annotations();
                while let Ok(first) = rx.recv() {
                    let mut jobs = vec![first];
                    while let Ok(j) = rx.try_recv() {
                        jobs.push(j);
                    }
                    // Newest first; skip zoom levels that are no longer wanted.
                    let mut seen = HashSet::new();
                    for (key, scale) in jobs.into_iter().rev() {
                        if !seen.insert(key) {
                            continue;
                        }
                        if key.level != THUMB && key.level != wanted.load(Ordering::Relaxed) {
                            let _ = dtx.send(Done { key, texture: None });
                            continue;
                        }
                        let draw = || {
                            let mut img = doc.render(key.page, scale)?;
                            if key.dark {
                                pdf::darken(&mut img);
                            }
                            let ci = egui::ColorImage::from_rgba_premultiplied([img.width, img.height], &img.rgba);
                            Some(ctx.load_texture(format!("p{}-{}", key.page, key.level), ci, TextureOptions::LINEAR))
                        };
                        // PDFKit leaves temporary objects behind: clear them per page.
                        #[cfg(target_os = "macos")]
                        let tex = objc2::rc::autoreleasepool(|_| draw());
                        #[cfg(not(target_os = "macos"))]
                        let tex = draw();
                        if dtx.send(Done { key, texture: tex }).is_err() {
                            return;
                        }
                        ctx.request_repaint();
                    }
                }
            })
            .expect("render thread");
        Renderer { tx, done, wanted_level, requested: HashSet::new() }
    }

    pub fn set_wanted_level(&self, level: u32) {
        self.wanted_level.store(level, Ordering::Relaxed);
    }

    /// Ask for a picture once; `forget` lets it be asked for again.
    pub fn request(&mut self, key: Key, scale: f64) {
        if self.requested.insert(key) {
            let _ = self.tx.send((key, scale));
        }
    }

    pub fn forget(&mut self, key: &Key) {
        self.requested.remove(key);
    }

    pub fn forget_all(&mut self) {
        self.requested.clear();
    }
}
