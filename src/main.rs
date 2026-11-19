// A window app: no black console window behind it on Windows.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod icons;
mod naming;
mod markdown;
mod model;
mod obsidian;
mod pdf;
mod render;
mod shortcuts;
mod theme;
mod ai;
mod citation;

use std::path::PathBuf;

/// A two-page PDF (English and Korean text) the tests work on.
#[cfg(test)]
pub const SAMPLE_PDF: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/sample.pdf");

use eframe::egui;

struct Runner {
    app: app::App,
    open: Vec<PathBuf>,
    screenshot: Option<PathBuf>,
    frames: u32,
    requested: bool,
}

impl eframe::App for Runner {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.app.ui(ui);
        let Some(out) = self.screenshot.clone() else { return };
        let ctx = ui.ctx().clone();
        self.frames += 1;
        // Look for the picture we asked for.
        let image = ctx.input(|i| {
            i.raw.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(img) = image {
            let [w, h] = img.size;
            let mut buf = image::RgbaImage::new(w as u32, h as u32);
            for (i, px) in img.pixels.iter().enumerate() {
                let [r, g, b, a] = px.to_srgba_unmultiplied();
                buf.put_pixel((i % w) as u32, (i / w) as u32, image::Rgba([r, g, b, a]));
            }
            if let Some(dir) = out.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Err(e) = buf.save(&out) {
                eprintln!("could not save screenshot: {e}");
            }
            std::process::exit(0);
        }
        // Ask at frame 120, and again every 90 frames if the picture hasn't come.
        if self.frames >= 120 && (!self.requested || (self.frames - 120) % 90 == 0) {
            self.requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
        }
        if self.frames > 900 {
            // The window isn't being drawn (covered by other windows): draw
            // the same window off-screen and save that instead.
            let code = match offscreen(self.open.clone(), &out) {
                Ok(()) => 0,
                Err(e) => {
                    eprintln!("screenshot never arrived and off-screen drawing failed: {e}");
                    1
                }
            };
            std::process::exit(code);
        }
        ctx.request_repaint();
    }
}

/// Draw the window without showing it (GPU, off-screen) and save a PNG.
fn offscreen(open: Vec<PathBuf>, out: &std::path::Path) -> Result<(), String> {
    let scratch = egui::Context::default();
    let app = app::App::new(&scratch, open, true);
    let mut ready = false;
    let mut h = egui_kittest::Harness::builder()
        .with_size(egui::Vec2::from(window_size()))
        .with_pixels_per_point(2.0)
        .wgpu()
        .build_ui_state(
            move |ui, app: &mut app::App| {
                if !ready {
                    app.prepare(ui.ctx());
                    ready = true;
                    return;
                }
                app.ui(ui)
            },
            app,
        );
    // Pages are drawn on a background thread: give it a moment.
    for _ in 0..60 {
        h.step();
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let img = h.render()?;
    if let Some(dir) = out.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    img.save(out).map_err(|e| e.to_string())
}

/// `PDF_NOTES_PHONE=1` (or `--phone`) shows the phone layout at iPhone size.
fn window_size() -> [f32; 2] {
    if std::env::var("PDF_NOTES_PHONE").is_ok() { [393.0, 852.0] } else { [1280.0, 820.0] }
}

/// `--make-icon <folder>`: draw the app icon at every size macOS wants
/// (an .iconset folder for `iconutil`).
fn make_icon(dir: &std::path::Path) -> Result<(), String> {
    let mut h = egui_kittest::Harness::builder().with_size(egui::Vec2::splat(1024.0)).with_pixels_per_point(1.0).wgpu().build_ui(|ui| {
        let r = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::Vec2::splat(1024.0));
        ui.painter().rect_filled(r, 0.0, egui::Color32::TRANSPARENT);
        icons::paint_logo(ui.painter(), r, true);
    });
    h.run_steps(2);
    let img = h.render()?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    for (name, size) in [
        ("icon_16x16", 16),
        ("icon_16x16@2x", 32),
        ("icon_32x32", 32),
        ("icon_32x32@2x", 64),
        ("icon_128x128", 128),
        ("icon_128x128@2x", 256),
        ("icon_256x256", 256),
        ("icon_256x256@2x", 512),
        ("icon_512x512", 512),
        ("icon_512x512@2x", 1024),
    ] {
        let small = image::imageops::resize(&img, size, size, image::imageops::FilterType::Lanczos3);
        small.save(dir.join(format!("{name}.png"))).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn main() -> eframe::Result<()> {
    {
        let args: Vec<String> = std::env::args().collect();
        if let Some(i) = args.iter().position(|a| a == "--make-icon") {
            let dir = args.get(i + 1).cloned().unwrap_or_else(|| "icon.iconset".into());
            match make_icon(std::path::Path::new(&dir)) {
                Ok(()) => std::process::exit(0),
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            }
        }
    }
    if std::env::args().any(|a| a == "--phone") {
        // SAFETY: set before any other thread starts.
        unsafe { std::env::set_var("PDF_NOTES_PHONE", "1") };
    }
    let mut screenshot: Option<PathBuf> = None;
    let mut open: Vec<PathBuf> = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == "--screenshot" {
            screenshot = args.next().map(PathBuf::from);
        } else if a == "--phone" {
        } else if !a.starts_with("-psn") {
            open.push(PathBuf::from(a));
        }
    }
    let shot = screenshot.is_some();

    let mut viewport = egui::ViewportBuilder::default()
        .with_title("PDF Notes")
        .with_icon(std::sync::Arc::new(eframe::icon_data::from_png_bytes(include_bytes!("../assets/AppIcon.iconset/icon_256x256@2x.png")).unwrap_or_default()))
        .with_inner_size(window_size())
        .with_min_inner_size([360.0, 480.0]);
    if shot {
        viewport = viewport.with_active(false);
        #[cfg(target_os = "windows")]
        {
            viewport = viewport.with_window_level(egui::WindowLevel::AlwaysOnBottom);
        }
    }
    let mut options = eframe::NativeOptions { viewport, ..Default::default() };
    #[cfg(target_os = "macos")]
    if shot {
        options.event_loop_builder = Some(Box::new(|builder| {
            use winit::platform::macos::EventLoopBuilderExtMacOS;
            builder.with_activate_ignoring_other_apps(false);
        }));
    }
    options.run_and_return = false;

    eframe::run_native(
        "PDF Notes",
        options,
        Box::new(move |cc| {
            let app = app::App::new(&cc.egui_ctx, open.clone(), shot);
            Ok(Box::new(Runner { app, open, screenshot, frames: 0, requested: false }))
        }),
    )
}

#[cfg(test)]
mod tests {
    use crate::model::*;
    use crate::{markdown, obsidian, pdf::PdfDoc};
    use std::path::Path;

    #[test]
    fn highlight_to_obsidian_round_trip() {
        let src = Path::new(crate::SAMPLE_PDF);
        if !src.exists() {
            return;
        }
        let work = std::env::temp_dir().join("pnt-test");
        let _ = std::fs::remove_dir_all(&work);
        std::fs::create_dir_all(work.join("vault/.obsidian")).unwrap();
        let pdf_path = work.join("paper.pdf");
        std::fs::copy(src, &pdf_path).unwrap();

        let doc = PdfDoc::open(&pdf_path).unwrap();
        assert_eq!(doc.page_count(), 2);
        let (w, _h) = doc.page_size(0);
        assert!((w - 612.0).abs() < 1.0);
        // Drag across the first body paragraph (view points, top-left origin).
        let (text, rects) = doc.select_text(0, (70.0, 135.0), (400.0, 175.0)).expect("selection");
        println!("SELECTED: {text:?} rects={}", rects.len());
        assert!(text.contains("Exosomes") || text.contains("vesicles"), "{text}");
        let (ko, _) = doc.select_text(0, (70.0, 200.0), (500.0, 220.0)).expect("korean");
        println!("KO: {ko:?}");

        let mut notes = DocNotes { pdf_path: pdf_path.to_string_lossy().into(), title: "Sample Paper".into(), author: "A. Author".into(), ..Default::default() };
        let mut a = Annotation::new(Kind::Highlight, 0, 2, rects.clone(), text.clone());
        a.comment = "Key idea #exosomes #Biomarker".into();
        notes.annotations.push(a);
        notes.annotations.push(Annotation::new(Kind::Underline, 0, 3, vec![ViewRect { x: 70.0, y: 200.0, w: 300.0, h: 14.0 }], ko.clone()));
        notes.annotations.push(Annotation::new(Kind::Strike, 1, 0, vec![ViewRect { x: 70.0, y: 260.0, w: 300.0, h: 14.0 }], "struck words".into()));

        let fmt = NoteFormat::default();
        let label = |i: usize| doc.page_label(i);
        let md = markdown::build(&notes, &fmt, &markdown::Target::Plain { pdf_path: "/x/paper.pdf" }, &label);
        println!("---- PLAIN ----\n{}", md.body);
        assert!(md.body.starts_with("# Sample Paper\n## A. Author\n"));
        assert!(md.body.contains("#### [Page 1](file:///x/paper.pdf#page=1)"));
        assert!(md.body.contains("> **"));
        assert!(md.body.contains("> <del>struck words</del>"));
        assert_eq!(md.tags, vec!["exosomes".to_string(), "biomarker".to_string()]);

        let mut o = ObsidianSettings::default();
        o.vault = work.join("vault").to_string_lossy().into();
        let r = obsidian::sync(&notes, &fmt, &o, &label).unwrap();
        let first = std::fs::read_to_string(&r.note).unwrap();
        println!("---- OBSIDIAN ----\n{first}");
        assert!(first.contains("[[Highlights/attachments/paper.pdf#page=1|Page 1]]"));
        assert!(work.join("vault/Highlights/attachments/paper.pdf").exists());
        // Person writes their own thoughts, then we sync again with a change.
        // …and a memo line inside the memo section, in Obsidian.
        std::fs::write(&r.note, first.replace("%% memo:end %%\n", "from obsidian\n%% memo:end %%\n\nMY OWN TEXT\n")).unwrap();
        notes.annotations[0].comment = "Changed comment".into();
        let r2 = obsidian::sync(&notes, &fmt, &o, &label).unwrap();
        assert_eq!(r2.memo, "from obsidian", "memo typed in Obsidian comes back");
        let second = std::fs::read_to_string(&r.note).unwrap();
        assert!(second.contains("MY OWN TEXT"));
        assert!(second.contains("from obsidian"));
        assert!(second.contains("Changed comment"));
        assert!(!second.contains("Key idea"));
        assert_eq!(second.matches("%% highlights:end %%").count(), 1);

        // Into the PDF and back.
        assert!(doc.save_with_annotations(&pdf_path, &notes.annotations, &PALETTE));
        let again = PdfDoc::open(&pdf_path).unwrap();
        let imported = again.take_existing_annotations();
        println!("IMPORTED {}: {:?}", imported.len(), imported.iter().map(|a| (a.kind, a.color, a.text.clone(), a.comment.clone())).collect::<Vec<_>>());
        assert_eq!(imported.len(), 3);
        assert_eq!(imported[0].color, 2);
        assert_eq!(imported[0].comment, "Changed comment");
    }
}

#[cfg(test)]
mod perf {
    #[test]
    #[ignore]
    fn timing() {
        use std::time::Instant;
        let p = std::path::Path::new("/tmp/pnt/heavy.pdf");
        let t = Instant::now();
        let d = crate::pdf::PdfDoc::open(p).unwrap();
        println!("open {:?}", t.elapsed());
        for i in 0..3 {
            let t = Instant::now();
            let mut img = d.render(i, 2.0).unwrap();
            let r = t.elapsed();
            let t = Instant::now();
            crate::pdf::darken(&mut img);
            println!("page {i}: render {:?} darken {:?} ({}x{})", r, t.elapsed(), img.width, img.height);
        }
        let t = Instant::now();
        for _ in 0..20 {
            let _ = d.select_text(1, (60.0, 100.0), (400.0, 300.0));
        }
        println!("select_text x20 {:?}", t.elapsed());
        let t = Instant::now();
        let _ = std::fs::read("/System/Library/Fonts/AppleSDGothicNeo.ttc");
        println!("read sdgothic {:?}", t.elapsed());
    }
}
