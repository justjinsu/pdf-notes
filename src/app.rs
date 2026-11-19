//! The window: toolbar and document tabs on top, page thumbnails on the left,
//! the PDF (one, or two side by side) in the middle and the notes panel on the
//! right — plus the annotation popovers, document info, settings and Obsidian.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontFamily, FontId, Id, Key, Margin, Order, Pos2, Rect, RichText, Sense,
    Stroke, TextureHandle, Vec2,
};

use crate::icons::{self, Icon};
use crate::markdown::{self, Target};
use crate::model::{images_dir, Annotation, DocNotes, Kind, LinkStyle, Settings, ViewRect, COLOR_NAMES_KO, PALETTE};
use crate::naming;
use crate::obsidian;
use crate::pdf::PdfDoc;
use crate::render::{self, Renderer};
use crate::shortcuts::{self, Action};
use crate::citation;
use crate::ai;
use std::sync::mpsc::Receiver;

use crate::theme::{self, ACCENT};
const LINK: Color32 = ACCENT;
const PAGE_GAP: f32 = 18.0;
const THUMB_W: f32 = 112.0;
const MIN_ZOOM: f32 = 0.3;
const MAX_ZOOM: f32 = 5.0;
/// Draw sharp pages only once the zoom has rested this long.
const ZOOM_SETTLE: Duration = Duration::from_millis(140);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tool {
    Select,
    Strike,
    Underline,
    Highlight,
    Note,
    Image,
}

impl Tool {
    const ALL: [Tool; 6] = [Tool::Select, Tool::Strike, Tool::Underline, Tool::Highlight, Tool::Note, Tool::Image];

    fn kind(self) -> Option<Kind> {
        match self {
            Tool::Select => None,
            Tool::Strike => Some(Kind::Strike),
            Tool::Underline => Some(Kind::Underline),
            Tool::Highlight => Some(Kind::Highlight),
            Tool::Note => Some(Kind::Note),
            Tool::Image => Some(Kind::Image),
        }
    }
    fn icon(self) -> Icon {
        match self {
            Tool::Select => Icon::Select,
            Tool::Strike => Icon::Strike,
            Tool::Underline => Icon::Underline,
            Tool::Highlight => Icon::Highlight,
            Tool::Note => Icon::Note,
            Tool::Image => Icon::Image,
        }
    }
    fn tip(self) -> &'static str {
        match self {
            Tool::Select => "선택 — 글을 끌어서 고른 뒤 종류를 고르세요",
            Tool::Strike => "취소선 — 글을 끌거나 단어를 클릭하세요",
            Tool::Underline => "밑줄 — 글을 끌거나 단어를 클릭하세요",
            Tool::Highlight => "형광펜 — 글을 끌거나 단어를 클릭하세요",
            Tool::Note => "메모 — 페이지를 클릭해 메모를 붙이세요",
            Tool::Image => "이미지 선택 — 영역을 끌어서 고르세요",
        }
    }
}

fn color32(i: usize) -> Color32 {
    let c = PALETTE.get(i).copied().unwrap_or([250, 236, 70]);
    Color32::from_rgb(c[0], c[1], c[2])
}

/// Text just selected with the Select tool, waiting for the popover choice.
struct Pending {
    page: usize,
    text: String,
    rects: Vec<ViewRect>,
    anchor: Pos2,
}

enum Drag {
    Text { page: usize, start: (f32, f32), now: (f32, f32) },
    Area { page: usize, start: (f32, f32), now: (f32, f32) },
}

/// One open document and everything about how it is being viewed.
struct Tab {
    path: PathBuf,
    pdf: PdfDoc,
    notes: DocNotes,
    sizes: Vec<(f32, f32)>,
    pdf_dirty: bool,
    renderer: Renderer,
    pages: HashMap<render::Key, TextureHandle>,
    thumbs: HashMap<render::Key, TextureHandle>,
    zoom: f32,
    zoom_changed: Instant,
    current_page: usize,
    scroll_to_page: Option<usize>,
    set_offset: Option<Vec2>,
    offset: Vec2,
    viewport: Rect,
    drag: Option<Drag>,
    sel_cache: Option<((usize, (f32, f32), (f32, f32)), Vec<ViewRect>)>,
    pending: Option<Pending>,
    editing: Option<(u64, Pos2)>,
    search_hits: Vec<usize>,
    search_index: usize,
    sync_status: Option<(bool, String)>,
    changed_at: Option<Instant>,
    rev: u64,
    md_cache: Option<(u64, String)>,
    /// CrossRef answer on its way.
    cite_rx: Option<Receiver<Result<citation::Found, String>>>,
    cite_status: Option<String>,
    /// Fit the page width to the view the next time it's drawn.
    fit_width: bool,
    last_width: f32,
    /// When the Obsidian note was last read or written by us.
    note_mtime: Option<std::time::SystemTime>,
    note_checked: Instant,
    /// Undo/redo of annotations: earlier states, and states undone.
    /// Conversation with the AI about this document.
    ai_history: Vec<ai::Message>,
    ai_input: String,
    ai_rx: Option<Receiver<Result<String, String>>>,
    ai_error: Option<String>,
    undo: Vec<Vec<Annotation>>,
    redo: Vec<Vec<Annotation>>,
    /// The annotations as of the last recorded step.
    snapshot: Vec<Annotation>,
    snapshot_at: Instant,
    #[cfg(test)]
    debug_first_page: Option<Rect>,
}

impl Tab {
    fn name(&self) -> String {
        self.path.file_name().map(|s| crate::pdf::nfc(&s.to_string_lossy())).unwrap_or_default()
    }

    fn markdown(&mut self, s: &Settings) -> String {
        let key = self.rev ^ settings_rev(s);
        if let Some((k, md)) = &self.md_cache {
            if *k == key {
                return md.clone();
            }
        }
        let pdf = &self.pdf;
        let label = |i: usize| pdf.page_label(i);
        let path = self.path.to_string_lossy().to_string();
        let mut md = markdown::build(&self.notes, &s.format, &Target::Plain { pdf_path: &path }, &label).body;
        if !self.notes.memo.trim().is_empty() {
            md.push_str(&format!("\n## 내 메모\n\n{}\n", self.notes.memo.trim_end()));
        }
        self.md_cache = Some((key, md.clone()));
        md
    }

    /// Text selection between two points, remembered while the mouse rests.
    fn select(&mut self, page: usize, a: (f32, f32), b: (f32, f32)) -> Option<Vec<ViewRect>> {
        let k = (page, a, b);
        if let Some((ck, rects)) = &self.sel_cache {
            if *ck == k {
                return Some(rects.clone());
            }
        }
        let rects = self.pdf.select_text(page, a, b).map(|(_, r)| r).unwrap_or_default();
        self.sel_cache = Some((k, rects.clone()));
        Some(rects)
    }
}

fn settings_rev(s: &Settings) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    serde_json::to_string(&s.format).unwrap_or_default().hash(&mut h);
    h.finish()
}

/// What the page view needs to know from the app.
#[derive(Clone, Copy)]
struct ViewCtx {
    tool: Tool,
    color: usize,
    dark_pages: bool,
    focused: bool,
    /// Phone: one finger scrolls; only marking tools select by dragging.
    touch: bool,
}

pub struct App {
    settings: Settings,
    tabs: Vec<Tab>,
    /// Which tab is shown in the left (or only) pane and in the right pane.
    left: usize,
    right: Option<usize>,
    focus_right: bool,
    tool: Tool,
    color: usize,
    show_sidebar: bool,
    show_notes: bool,
    notes_source: bool,
    show_meta: bool,
    show_settings: bool,
    settings_tab: u8,
    search: String,
    status: String,
    vaults: Vec<String>,
    screenshot_mode: bool,
    /// Waiting for the key combination of this action (in Settings).
    recording: Option<Action>,
    /// Light or dark as shown now (system setting or the person's choice).
    applied_dark: bool,
    /// Right panel shows the AI conversation instead of the notes.
    panel_ai: bool,
    ai_status: Vec<(ai::Provider, Result<String, String>)>,
    ai_key_input: String,
    /// The message shown as a fading note, and when it appeared.
    toast_text: String,
    toast_at: Instant,
    /// Phone layout: forced (iPhone, `--phone`) or because the window is narrow.
    force_phone: bool,
    phone_notes: bool,
    ctx: egui::Context,
    #[cfg(test)]
    debug_page_rect: Option<Rect>,
}

impl App {
    pub fn new(ctx: &egui::Context, open: Vec<PathBuf>, screenshot_mode: bool) -> Self {
        install_fonts(ctx);
        let mut settings = if screenshot_mode { Settings::default() } else { Settings::load() };
        if screenshot_mode {
            if let Ok(v) = std::env::var("PDF_NOTES_DARK") {
                settings.dark = v == "1";
                settings.follow_system = false;
            }
        }
        let mut app = App {
            color: settings.default_color,
            settings,
            tabs: Vec::new(),
            left: 0,
            right: None,
            focus_right: false,
            tool: Tool::Select,
            show_sidebar: true,
            show_notes: true,
            notes_source: false,
            show_meta: false,
            show_settings: false,
            settings_tab: 0,
            search: String::new(),
            status: String::new(),
            vaults: obsidian::known_vaults(),
            screenshot_mode,
            recording: None,
            applied_dark: false,
            panel_ai: false,
            ai_status: Vec::new(),
            ai_key_input: String::new(),
            toast_text: String::new(),
            toast_at: Instant::now(),
            force_phone: cfg!(target_os = "ios") || std::env::var("PDF_NOTES_PHONE").is_ok(),
            phone_notes: false,
            ctx: ctx.clone(),
            #[cfg(test)]
            debug_page_rect: None,
        };
        if app.settings.ai.provider.is_none() && !screenshot_mode {
            app.settings.ai.provider = ai::first_ready();
        }
        app.applied_dark = app.want_dark(ctx);
        theme::apply(ctx, app.applied_dark);
        if app.settings.obsidian.vault.is_empty() {
            if let Some(v) = app.vaults.first() {
                app.settings.obsidian.vault = v.clone();
            }
        }
        let two = open.len() >= 2;
        for p in open {
            app.open(&p);
        }
        // For checking the look in screenshots only (never saved).
        if screenshot_mode && std::env::var("PDF_NOTES_DEMO").is_ok() {
            if let Some(t) = app.tabs.first_mut() {
                let picks = [((70.0, 140.0), (420.0, 160.0), Kind::Highlight, 2usize, "핵심 정의 #exosome"), ((70.0, 205.0), (480.0, 222.0), Kind::Underline, 4, ""), ((70.0, 300.0), (380.0, 318.0), Kind::Highlight, 3, "")];
                for (a, b, k, c, comment) in picks {
                    if let Some((text, rects)) = t.pdf.select_text(0, a, b) {
                        let mut an = Annotation::new(k, 0, c, rects, text);
                        an.comment = comment.into();
                        t.notes.annotations.push(an);
                    }
                }
                t.notes.memo = "2장 방법론이 우리 실험과 비슷함 — 다음 주 미팅에서 공유".into();
                if std::env::var("PDF_NOTES_PANEL").map(|v| v == "ai").unwrap_or(false) {
                    t.ai_history.push(ai::Message { from_ai: false, text: ai::QUICK[0].1.to_string() });
                    t.ai_history.push(ai::Message {
                        from_ai: true,
                        text: "**한 줄 요지**: 혈액 속 엑소좀은 조직검사 없이 암을 추적할 수 있는 표지자다 (p. 1).\n\n## 핵심 주장\n- 엑소좀은 대부분의 세포가 분비하며 단백질·지질·핵산을 운반한다 (p. 1)\n- 액체생검은 정확도를 높이고 지속적인 관찰을 가능하게 한다 (p. 1)\n\n## 한계\n- 표본 수와 검증 방법은 이 문서에 나와 있지 않아요.".into(),
                    });
                    app.panel_ai = true;
                }
            }
        }
        if two && screenshot_mode {
            app.left = 0;
            app.right = Some(1);
        }
        app
    }

    /// Fonts and colours for a context this app draws into (used by the
    /// off-screen picture when the real window can't be photographed).
    pub fn prepare(&mut self, ctx: &egui::Context) {
        install_fonts(ctx);
        theme::apply(ctx, self.applied_dark);
        // Page pictures must belong to the context that shows them.
        self.ctx = ctx.clone();
        for t in &mut self.tabs {
            t.renderer = Renderer::start(t.path.clone(), ctx.clone());
            t.pages.clear();
            t.thumbs.clear();
        }
    }

    fn active(&self) -> usize {
        if self.focus_right { self.right.unwrap_or(self.left) } else { self.left }
    }

    fn tab(&mut self) -> Option<&mut Tab> {
        let i = self.active();
        self.tabs.get_mut(i)
    }

    fn tab_ref(&self) -> Option<&Tab> {
        self.tabs.get(self.active())
    }

    fn dark_pages(&self) -> bool {
        self.applied_dark && self.settings.dark_pdf
    }

    // ----------------------------------------------------------------- files

    /// Open a PDF in a new tab (or show its tab if it's already open).
    fn open(&mut self, path: &Path) {
        if let Some(i) = self.tabs.iter().position(|t| t.path == path) {
            self.show_tab(i);
            return;
        }
        let Some(pdf) = PdfDoc::open(path) else {
            self.status = match crate::pdf::engine_problem() {
                Some(why) => why,
                None => format!("PDF를 열 수 없어요: {}", path.display()),
            };
            return;
        };
        let imported = pdf.take_existing_annotations();
        let mut notes = DocNotes::load(path).unwrap_or_else(|| DocNotes {
            pdf_path: path.to_string_lossy().to_string(),
            title: pdf.title().unwrap_or_default(),
            author: pdf.author().unwrap_or_default(),
            ..Default::default()
        });
        notes.pdf_path = path.to_string_lossy().to_string();
        let mut added = 0;
        for a in imported {
            let dup = notes.annotations.iter().any(|b| {
                b.page == a.page && b.kind == a.kind && a.rects.iter().any(|r| b.rects.iter().any(|q| q.overlap_ratio(r) > 0.6))
            });
            if !dup {
                notes.annotations.push(a);
                added += 1;
            }
        }
        let mut named = false;
        if notes.note_name.trim().is_empty() {
            fill_details(&pdf, &mut notes, &self.settings.obsidian.name_template);
            named = true;
        }
        let sizes = (0..pdf.page_count())
            .map(|i| {
                let (w, h) = pdf.page_size(i);
                (w as f32, h as f32)
            })
            .collect();
        let mut msgs = Vec::new();
        if named && !notes.note_name.is_empty() {
            msgs.push(format!("노트 이름 · {}", notes.note_name));
        }
        if added > 0 {
            msgs.push(format!("주석 {added}개를 가져왔어요"));
        }
        self.status = msgs.join("  ·  ");
        if !self.screenshot_mode {
            self.settings.remember(path);
            self.settings.save();
            notes.save(path);
        }
        let renderer = Renderer::start(path.to_path_buf(), self.ctx.clone());
        let want_cite = self.settings.format.citation.auto_fetch && !self.screenshot_mode && !notes.doi.trim().is_empty() && notes.journal.trim().is_empty() && notes.authors.is_empty();
        let cite_rx = if want_cite { Some(start_cite_fetch(&notes.doi, &self.ctx)) } else { None };
        self.tabs.push(Tab {
            path: path.to_path_buf(),
            pdf,
            notes,
            sizes,
            pdf_dirty: false,
            renderer,
            pages: HashMap::new(),
            thumbs: HashMap::new(),
            zoom: 1.0,
            zoom_changed: Instant::now() - ZOOM_SETTLE,
            current_page: 0,
            scroll_to_page: Some(0),
            set_offset: None,
            offset: Vec2::ZERO,
            viewport: Rect::NOTHING,
            drag: None,
            sel_cache: None,
            pending: None,
            editing: None,
            search_hits: Vec::new(),
            search_index: 0,
            sync_status: None,
            changed_at: None,
            rev: 1,
            md_cache: None,
            cite_rx,
            cite_status: None,
            fit_width: true,
            last_width: 0.0,
            note_mtime: None,
            note_checked: Instant::now(),
            ai_history: Vec::new(),
            ai_input: String::new(),
            ai_rx: None,
            ai_error: None,
            undo: Vec::new(),
            redo: Vec::new(),
            snapshot: Vec::new(),
            snapshot_at: Instant::now(),
            #[cfg(test)]
            debug_first_page: None,
        });
        let i = self.tabs.len() - 1;
        self.tabs[i].snapshot = self.tabs[i].notes.annotations.clone();
        self.show_tab(i);
    }

    /// Show tab `i` in the focused pane.
    fn show_tab(&mut self, i: usize) {
        if self.right.is_some() && self.focus_right {
            if Some(i) != Some(self.left) {
                self.right = Some(i);
            } else {
                self.focus_right = false;
            }
        } else if self.right == Some(i) {
            self.focus_right = true;
        } else {
            self.left = i;
        }
    }

    fn close_tab(&mut self, i: usize) {
        if i >= self.tabs.len() {
            return;
        }
        let screenshot = self.screenshot_mode;
        flush(&mut self.tabs[i], &self.settings, screenshot, true);
        self.tabs.remove(i);
        let fix = |x: usize| if x > i { x - 1 } else { x };
        if self.right == Some(i) {
            self.right = None;
            self.focus_right = false;
        }
        if self.left == i {
            if let Some(r) = self.right.take() {
                self.left = r;
            }
            self.focus_right = false;
        }
        self.left = fix(self.left).min(self.tabs.len().saturating_sub(1));
        self.right = self.right.map(fix);
        if self.right == Some(self.left) {
            self.right = None;
            self.focus_right = false;
        }
    }

    fn toggle_split(&mut self) {
        if self.right.is_some() {
            self.right = None;
            self.focus_right = false;
            self.refit();
            return;
        }
        if self.tabs.len() < 2 {
            self.status = "나란히 보려면 PDF를 하나 더 여세요.".into();
            self.open_dialog();
            if self.tabs.len() < 2 {
                return;
            }
        }
        let other = (0..self.tabs.len()).find(|&j| j != self.left && j > self.left).or_else(|| (0..self.tabs.len()).find(|&j| j != self.left));
        self.right = other;
        self.focus_right = true;
        self.refit();
    }

    fn refit(&mut self) {
        for t in &mut self.tabs {
            t.fit_width = true;
        }
    }

    fn open_dialog(&mut self) {
        if let Some(files) = rfd::FileDialog::new().add_filter("PDF", &["pdf"]).pick_files() {
            for p in files {
                self.open(&p);
            }
        }
    }

    fn sync_active(&mut self, open_after: bool) {
        let screenshot = self.screenshot_mode;
        let i = self.active();
        if let Some(t) = self.tabs.get_mut(i) {
            sync_obsidian(t, &self.settings, screenshot, open_after);
        }
    }

    fn save_pdf_active(&mut self) {
        let screenshot = self.screenshot_mode;
        let i = self.active();
        if let Some(t) = self.tabs.get_mut(i) {
            t.pdf_dirty = true;
            if let Some(msg) = save_pdf(t, &self.settings, screenshot) {
                self.status = msg;
            }
            flush(t, &self.settings, screenshot, false);
        }
    }

    fn export_markdown(&mut self) {
        let settings = self.settings.clone();
        let Some(t) = self.tab() else { return };
        let name = format!("{}.md", obsidian::safe_name(&obsidian::note_title(&t.notes)));
        let mut dlg = rfd::FileDialog::new().set_file_name(&name).add_filter("Markdown", &["md"]);
        if let Some(dir) = t.path.parent() {
            dlg = dlg.set_directory(dir);
        }
        if let Some(out) = dlg.save_file() {
            let md = t.markdown(&settings);
            if let Some(parent) = out.parent() {
                for a in &t.notes.annotations {
                    if let Some(f) = &a.image_file {
                        let _ = std::fs::create_dir_all(parent.join("assets"));
                        let _ = std::fs::copy(images_dir().join(f), parent.join("assets").join(f));
                    }
                }
            }
            self.status = match std::fs::write(&out, md) {
                Ok(_) => format!("Markdown으로 내보냈어요: {}", out.display()),
                Err(e) => format!("내보내지 못했어요: {e}"),
            };
        }
    }

    fn run_search(&mut self) {
        let q = self.search.trim().to_lowercase();
        let shown = self.search.trim().to_string();
        let Some(t) = self.tab() else { return };
        t.search_hits.clear();
        t.search_index = 0;
        if q.is_empty() {
            return;
        }
        for p in 0..t.pdf.page_count() {
            if t.pdf.page_text(p).to_lowercase().contains(&q) {
                t.search_hits.push(p);
            }
        }
        let msg = if let Some(p) = t.search_hits.first() {
            t.scroll_to_page = Some(*p);
            format!("“{}”: {}개 페이지에서 찾았어요. Enter로 다음.", shown, t.search_hits.len())
        } else {
            format!("“{}”을(를) 찾지 못했어요.", shown)
        };
        self.status = msg;
    }

    fn clear_textures(&mut self) {
        for t in &mut self.tabs {
            t.pages.clear();
            t.thumbs.clear();
            t.renderer.forget_all();
        }
    }

    /// Light or dark: the system's when following it, else the choice.
    fn want_dark(&self, ctx: &egui::Context) -> bool {
        if self.settings.follow_system {
            match ctx.system_theme() {
                Some(egui::Theme::Dark) => true,
                Some(egui::Theme::Light) => false,
                None => self.settings.dark,
            }
        } else {
            self.settings.dark
        }
    }

    fn set_dark(&mut self, ctx: &egui::Context, dark: bool) {
        self.settings.dark = dark;
        self.settings.follow_system = false;
        self.applied_dark = dark;
        theme::apply(ctx, dark);
        self.clear_textures();
        if !self.screenshot_mode {
            self.settings.save();
        }
    }

    fn set_zoom(&mut self, z: f32) {
        if let Some(t) = self.tab() {
            let z = z.clamp(MIN_ZOOM, MAX_ZOOM);
            if (z - t.zoom).abs() > f32::EPSILON {
                // Keep the middle of the view in place.
                let anchor = t.viewport.center();
                zoom_about(t, z, anchor);
            }
        }
    }

    // ---------------------------------------------------------------- toolbar

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let pal = theme::pal(&ctx);
        let has_doc = !self.tabs.is_empty();
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            if icons::button(ui, Icon::SidebarLeft, self.show_sidebar, ACCENT, "페이지 목록").clicked() {
                self.show_sidebar = !self.show_sidebar;
            }
            if icons::button(ui, Icon::Folder, false, ACCENT, &crate::shortcuts::plat("PDF 열기  ⌘O")).clicked() {
                self.open_dialog();
            }
            ui.add_space(4.0);
            ui.add_enabled_ui(has_doc, |ui| {
                if icons::button(ui, Icon::Prev, false, ACCENT, "이전 쪽").clicked() {
                    if let Some(t) = self.tab() {
                        if t.current_page > 0 {
                            t.scroll_to_page = Some(t.current_page - 1);
                        }
                    }
                }
                if icons::button(ui, Icon::Next, false, ACCENT, "다음 쪽").clicked() {
                    if let Some(t) = self.tab() {
                        t.scroll_to_page = Some(t.current_page + 1);
                    }
                }
            });
            ui.add_space(8.0);
            let title_w = (ui.available_width() * 0.22).clamp(110.0, 220.0);
            ui.allocate_ui(Vec2::new(title_w, 36.0), |ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    ui.set_width(title_w);
                    match self.tab_ref() {
                        Some(t) => {
                            ui.add(egui::Label::new(RichText::new(t.name()).family(icons::bold()).size(13.0)).truncate());
                            ui.label(RichText::new(format!("{} / {}", t.current_page + 1, t.pdf.page_count())).size(11.0).color(pal.weak));
                        }
                        None => {
                            ui.label(RichText::new("PDF Notes").family(icons::bold()).size(13.0));
                        }
                    }
                });
            });

            // Annotation tools and the colour, as one quiet group.
            ui.add_enabled_ui(has_doc, |ui| {
                egui::Frame::new().fill(ui.visuals().faint_bg_color).corner_radius(CornerRadius::same(9)).inner_margin(Margin::symmetric(3, 2)).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.x = 1.0;
                    for t in Tool::ALL {
                        let key = self.settings.keymap.label(tool_action(t));
                        if icons::button_hint(ui, t.icon(), self.tool == t, ACCENT, t.tip(), &key).clicked() {
                            self.tool_clicked(t);
                        }
                    }
                    ui.add_space(3.0);
                    self.color_menu(ui);
                });
            });

            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                if icons::button(ui, Icon::SidebarRight, self.show_notes, ACCENT, "노트").clicked() {
                    self.show_notes = !self.show_notes;
                }
                let more = icons::button(ui, Icon::More, false, ACCENT, "더 보기");
                egui::Popup::menu(&more).show(|ui| {
                    ui.set_min_width(220.0);
                    ui.add_enabled_ui(has_doc, |ui| {
                        if ui.button("문서 정보…").clicked() {
                            self.show_meta = true;
                        }
                        ui.separator();
                        if ui.button("Markdown으로 저장…").clicked() {
                            self.export_markdown();
                        }
                        if ui.button("Markdown 복사").clicked() {
                            let s = self.settings.clone();
                            if let Some(t) = self.tab() {
                                let md = t.markdown(&s);
                                ui.ctx().copy_text(md);
                            }
                            self.status = "Markdown을 복사했어요".into();
                        }
                        if ui.button(crate::shortcuts::plat("PDF에 주석 저장  ⌘S")).clicked() {
                            self.save_pdf_active();
                        }
                    });
                    ui.separator();
                    if ui.button("AI 설정…").clicked() {
                        self.show_settings = true;
                        self.settings_tab = 6;
                    }
                    if ui.button("설정…").clicked() {
                        self.show_settings = true;
                    }
                });
                let dark = self.applied_dark;
                if icons::button(ui, if dark { Icon::Sun } else { Icon::Moon }, false, ACCENT, if dark { "밝은 화면으로" } else { "어두운 화면으로" }).clicked() {
                    self.set_dark(&ctx, !dark);
                }
                ui.add_enabled_ui(has_doc, |ui| {
                    let synced = self.tab_ref().and_then(|t| t.sync_status.as_ref()).map(|(ok, _)| *ok).unwrap_or(false);
                    let r = icons::button(ui, Icon::Gem, synced, ACCENT, "Obsidian에서 열기  (노트를 쓰고 엽니다)");
                    if r.clicked() {
                        self.sync_active(true);
                    }
                    if icons::button(ui, Icon::Split, self.right.is_some(), ACCENT, "나란히 보기").clicked() {
                        self.toggle_split();
                    }
                });
                ui.add_space(6.0);
                let search_w = (ui.available_width() - 16.0).clamp(90.0, 180.0);
                egui::Frame::new().fill(ui.visuals().faint_bg_color).corner_radius(CornerRadius::same(8)).inner_margin(Margin::symmetric(8, 4)).show(ui, |ui| {
                    ui.set_width(search_w);
                    ui.with_layout(egui::Layout::left_to_right(Align::Center), |ui| {
                        let (r, _) = ui.allocate_exact_size(Vec2::new(14.0, 16.0), Sense::hover());
                        icons::paint(ui.painter(), r, Icon::Search, pal.weak, ui.visuals().faint_bg_color);
                        let resp = ui.add(egui::TextEdit::singleline(&mut self.search).hint_text("검색").frame(egui::Frame::NONE).desired_width(f32::INFINITY));
                        if resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                            let n = self.tab_ref().map(|t| t.search_hits.len()).unwrap_or(0);
                            if n > 1 {
                                if let Some(t) = self.tab() {
                                    t.search_index = (t.search_index + 1) % n;
                                    t.scroll_to_page = Some(t.search_hits[t.search_index]);
                                }
                                resp.request_focus();
                            } else {
                                self.run_search();
                            }
                        }
                        if resp.changed() {
                            if let Some(t) = self.tab() {
                                t.search_hits.clear();
                            }
                        }
                    });
                });
            });
        });
    }

    /// Tabs for the open documents.
    fn tab_bar(&mut self, ui: &mut egui::Ui) {
        let mut select: Option<usize> = None;
        let mut close: Option<usize> = None;
        let active = self.active();
        egui::ScrollArea::horizontal().id_salt("tabs").scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden).show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                for (i, t) in self.tabs.iter().enumerate() {
                    let is_active = i == active;
                    let shown = i == self.left || Some(i) == self.right;
                    let fill = if is_active {
                        ui.visuals().extreme_bg_color
                    } else if shown {
                        ui.visuals().faint_bg_color
                    } else {
                        Color32::TRANSPARENT
                    };
                    egui::Frame::new()
                        .fill(fill)
                        .corner_radius(CornerRadius::same(8))
                        .stroke(if is_active { Stroke::new(1.0, ACCENT.gamma_multiply(0.6)) } else { Stroke::NONE })
                        .inner_margin(Margin { left: 10, right: 4, top: 3, bottom: 3 })
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            let mut label = RichText::new(truncate(&t.name(), 28)).size(12.5);
                            if is_active {
                                label = label.family(icons::bold());
                            } else {
                                label = label.color(ui.visuals().weak_text_color());
                            }
                            let r = ui.add(egui::Label::new(label).sense(Sense::click())).on_hover_text(t.path.to_string_lossy());
                            if r.clicked() {
                                select = Some(i);
                            }
                            let (cr, cresp) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::click());
                            if cresp.hovered() {
                                ui.painter().rect_filled(cr, 4.0, ui.visuals().widgets.hovered.weak_bg_fill);
                            }
                            icons::paint(ui.painter(), cr.shrink(4.0), Icon::Close, ui.visuals().weak_text_color(), fill);
                            if cresp.on_hover_text("닫기").clicked() {
                                close = Some(i);
                            }
                        });
                }
                let (r, resp) = ui.allocate_exact_size(Vec2::splat(24.0), Sense::click());
                if resp.hovered() {
                    ui.painter().rect_filled(r, 6.0, ui.visuals().widgets.hovered.weak_bg_fill);
                }
                ui.painter().text(r.center(), Align2::CENTER_CENTER, "+", FontId::proportional(17.0), ui.visuals().text_color());
                if resp.on_hover_text("PDF 더 열기").clicked() {
                    self.open_dialog();
                }
            });
        });
        if let Some(i) = select {
            self.show_tab(i);
        }
        if let Some(i) = close {
            self.close_tab(i);
        }
    }

    /// A toolbar tool was pressed. With text already selected, it marks that
    /// text right away (what the person expects from the highlight button).
    fn tool_clicked(&mut self, tool: Tool) {
        let color = self.color;
        let user = self.settings.format.user_name.clone();
        let screenshot = self.screenshot_mode;
        if let Some(t) = self.tab() {
            if t.pending.is_some() {
                match tool {
                    Tool::Highlight | Tool::Underline | Tool::Strike | Tool::Note => {
                        apply_pending(t, tool.kind().unwrap(), color, &user, screenshot);
                        return;
                    }
                    _ => t.pending = None,
                }
            }
            t.editing = None;
        }
        self.tool = tool;
    }

    fn color_menu(&mut self, ui: &mut egui::Ui) {
        let (r, resp) = ui.allocate_exact_size(Vec2::new(30.0, 28.0), Sense::click());
        if resp.hovered() {
            ui.painter().rect_filled(r, 8.0, ui.visuals().widgets.hovered.weak_bg_fill);
        }
        ui.painter().circle_filled(r.center(), 7.0, color32(self.color));
        ui.painter().circle_stroke(r.center(), 7.0, Stroke::new(1.0, Color32::from_black_alpha(40)));
        let resp = resp.on_hover_text(format!("색: {}", COLOR_NAMES_KO[self.color]));
        egui::Popup::menu(&resp).show(|ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                for i in 0..8 {
                    if color_dot(ui, i, self.color == i).clicked() {
                        self.color = i;
                    }
                }
            });
        });
    }

    // ---------------------------------------------------------------- sidebar

    fn sidebar(&mut self, ui: &mut egui::Ui) {
        let dark = self.dark_pages();
        let Some(t) = self.tab() else {
            ui.add_space(12.0);
            ui.vertical_centered(|ui| ui.label(RichText::new("PDF를 열면\n페이지가 여기에 보여요.").weak()));
            return;
        };
        while let Ok(done) = t.renderer.done.try_recv() {
            store_done(t, done);
        }
        let count = t.pdf.page_count();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.add_space(12.0);
            for p in 0..count {
                let (w, h) = t.sizes[p];
                let th = THUMB_W * h / w.max(1.0);
                ui.vertical_centered(|ui| {
                    let (rect, resp) = ui.allocate_exact_size(Vec2::new(THUMB_W, th), Sense::click());
                    if ui.is_rect_visible(rect) {
                        let key = render::Key { page: p, level: render::THUMB, dark };
                        if !t.thumbs.contains_key(&key) {
                            t.renderer.request(key, (THUMB_W * 2.0 / w.max(1.0)) as f64);
                        }
                        let painter = ui.painter();
                        painter.rect_filled(rect.translate(Vec2::new(0.0, 2.0)).expand(1.0), 4.0, Color32::from_black_alpha(28));
                        if let Some(tex) = t.thumbs.get(&key) {
                            painter.image(tex.id(), rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
                        } else {
                            painter.rect_filled(rect, 2.0, if dark { Color32::from_gray(30) } else { Color32::WHITE });
                        }
                        let mut x = rect.right() - 7.0;
                        let mut seen = Vec::new();
                        for a in t.notes.annotations.iter().filter(|a| a.page == p) {
                            if !seen.contains(&a.color) {
                                seen.push(a.color);
                                painter.circle_filled(Pos2::new(x, rect.top() + 7.0), 3.5, color32(a.color));
                                x -= 8.0;
                            }
                        }
                        if p == t.current_page {
                            painter.rect_stroke(rect.expand(3.0), 5.0, Stroke::new(2.5, ACCENT), egui::StrokeKind::Outside);
                        }
                    }
                    if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        t.scroll_to_page = Some(p);
                    }
                    ui.add_space(2.0);
                    let label = RichText::new(t.pdf.page_label(p)).size(11.0);
                    ui.label(if p == t.current_page { label.color(ACCENT).family(icons::bold()) } else { label.weak() });
                });
                ui.add_space(8.0);
            }
        });
    }

    // ---------------------------------------------------------------- welcome

    fn welcome(&mut self, ui: &mut egui::Ui) {
        let pal = theme::pal(ui.ctx());
        let avail = ui.available_rect_before_wrap();
        let dropping = ui.ctx().input(|i| !i.raw.hovered_files.is_empty());
        if dropping {
            ui.painter().rect_stroke(avail.shrink(10.0), 14.0, Stroke::new(2.0, pal.accent), egui::StrokeKind::Inside);
        }
        let col_w = 380.0f32.min(avail.width() - 40.0);
        let col = Rect::from_center_size(Pos2::new(avail.center().x, avail.top() + avail.height() * 0.5), Vec2::new(col_w, avail.height() - 40.0));
        ui.scope_builder(egui::UiBuilder::new().max_rect(col), |ui| {
            ui.vertical_centered(|ui| {
                let recent: Vec<String> = self.settings.recent.iter().filter(|r| Path::new(r).exists()).take(6).cloned().collect();
                let content_h = 250.0 + if recent.is_empty() { 0.0 } else { 30.0 + recent.len() as f32 * 34.0 };
                ui.add_space(((col.height() - content_h) / 2.0).max(10.0));
                let (lr, _) = ui.allocate_exact_size(Vec2::splat(76.0), Sense::hover());
                icons::paint_logo(ui.painter(), lr, true);
                ui.add_space(6.0);
                ui.label(RichText::new("PDF Notes").family(icons::bold()).size(22.0));
                ui.add_space(2.0);
                ui.label(RichText::new("읽고, 표시하고, Obsidian에 남기세요").size(13.0).color(pal.weak));
                ui.add_space(22.0);
                let open = egui::Button::new(RichText::new("PDF 열기…").family(icons::bold()).size(13.5).color(Color32::WHITE))
                    .fill(pal.accent)
                    .corner_radius(CornerRadius::same(8))
                    .min_size(Vec2::new(180.0, 36.0));
                if ui.add(open).clicked() {
                    self.open_dialog();
                }
                ui.add_space(6.0);
                ui.label(RichText::new("또는 이 창에 끌어다 놓기").size(11.5).color(pal.weak));

                if !recent.is_empty() {
                    ui.add_space(26.0);
                    ui.with_layout(egui::Layout::top_down(Align::Min), |ui| {
                        ui.label(RichText::new("최근 문서").family(icons::bold()).size(11.5).color(pal.weak));
                        ui.add_space(4.0);
                        for r in recent {
                            let p = PathBuf::from(&r);
                            let name = p.file_name().map(|s| crate::pdf::nfc(&s.to_string_lossy())).unwrap_or_default();
                            let folder = p.parent().and_then(|d| d.file_name()).map(|s| crate::pdf::nfc(&s.to_string_lossy())).unwrap_or_default();
                            let (row, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 32.0), Sense::click());
                            if resp.hovered() {
                                ui.painter().rect_filled(row, 7.0, ui.visuals().faint_bg_color);
                            }
                            let ir = Rect::from_center_size(Pos2::new(row.left() + 16.0, row.center().y), Vec2::splat(16.0));
                            icons::paint(ui.painter(), ir, Icon::Page, pal.weak, pal.canvas);
                            ui.painter().text(Pos2::new(row.left() + 32.0, row.center().y), Align2::LEFT_CENTER, truncate(&name, 34), FontId::proportional(13.0), pal.text);
                            ui.painter().text(Pos2::new(row.right() - 8.0, row.center().y), Align2::RIGHT_CENTER, truncate(&folder, 16), FontId::proportional(11.5), pal.weak);
                            if resp.on_hover_text(&r).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                                self.open(&p);
                            }
                        }
                    });
                }
            });
        });
        // Where notes go, quietly at the bottom.
        let o = &self.settings.obsidian;
        let line = if o.vault.is_empty() { "Obsidian 보관함을 고르세요".to_string() } else { format!("{} › {}에 노트가 저장돼요", obsidian::vault_name(&o.vault), o.notes_folder) };
        let bottom = Rect::from_center_size(Pos2::new(avail.center().x, avail.bottom() - 24.0), Vec2::new(avail.width(), 20.0));
        let resp = ui.interact(bottom.shrink2(Vec2::new(avail.width() * 0.3, 0.0)), Id::new("welcome-vault"), Sense::click());
        let gem = Rect::from_center_size(Pos2::new(bottom.center().x - line.chars().count() as f32 * 3.3 - 12.0, bottom.center().y), Vec2::splat(13.0));
        icons::paint(ui.painter(), gem, Icon::Gem, pal.accent, pal.canvas);
        ui.painter().text(bottom.center(), Align2::CENTER_CENTER, line, FontId::proportional(11.5), if resp.hovered() { pal.text } else { pal.weak });
        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            self.show_settings = true;
            self.settings_tab = 1;
        }
    }

    // --------------------------------------------------------------- popovers

    fn pending_popover(&mut self, ctx: &egui::Context) {
        let color = self.color;
        let user = self.settings.format.user_name.clone();
        let screenshot = self.screenshot_mode;
        let mut new_color = None;
        let mut msg = None;
        let Some(t) = self.tab() else { return };
        let Some(pend) = &t.pending else { return };
        let anchor = pend.anchor + Vec2::new(0.0, 14.0);
        let mut choice: Option<Kind> = None;
        let mut copy = false;
        egui::Area::new(Id::new("pending"))
            .order(Order::Foreground)
            .fixed_pos(anchor)
            .pivot(Align2::CENTER_TOP)
            .constrain(true)
            .show(ctx, |ui| {
                popover_frame(ui).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        if icons::button(ui, Icon::Copy, false, ACCENT, "복사").clicked() {
                            copy = true;
                        }
                        ui.add_space(6.0);
                        for (icon, k, tip) in [
                            (Icon::Strike, Kind::Strike, "취소선으로"),
                            (Icon::Underline, Kind::Underline, "밑줄로"),
                            (Icon::Highlight, Kind::Highlight, "형광펜으로"),
                            (Icon::Note, Kind::Note, "형광펜 + 메모"),
                        ] {
                            if icons::button(ui, icon, false, ACCENT, tip).clicked() {
                                choice = Some(k);
                            }
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        for i in 0..8 {
                            if color_dot(ui, i, color == i).clicked() {
                                new_color = Some(i);
                            }
                        }
                    });
                });
            });
        if copy {
            ctx.copy_text(pend.text.clone());
            t.pending = None;
            msg = Some("복사했어요.".to_string());
        } else if let Some(k) = choice {
            apply_pending(t, k, new_color.unwrap_or(color), &user, screenshot);
        } else if ctx.input(|i| i.key_pressed(Key::Escape)) {
            t.pending = None;
        }
        if let Some(c) = new_color {
            self.color = c;
        }
        if let Some(m) = msg {
            self.status = m;
        }
    }

    fn edit_popover(&mut self, ctx: &egui::Context) {
        let user = self.settings.format.user_name.clone();
        let screenshot = self.screenshot_mode;
        let Some(t) = self.tab() else { return };
        let Some((id, anchor)) = t.editing else { return };
        let Some(mut a) = t.notes.annotations.iter().find(|a| a.id == id).cloned() else {
            t.editing = None;
            return;
        };
        let before = (a.kind, a.color, a.comment.clone());
        let mut delete = false;
        let mut close = false;
        egui::Area::new(Id::new("edit"))
            .order(Order::Foreground)
            .fixed_pos(anchor + Vec2::new(0.0, 16.0))
            .pivot(Align2::CENTER_TOP)
            .constrain(true)
            .show(ctx, |ui| {
                popover_frame(ui).show(ui, |ui| {
                    ui.set_width(320.0);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        if icons::button(ui, Icon::Copy, false, ACCENT, "내용 복사").clicked() {
                            let txt = if a.comment.is_empty() { a.text.clone() } else { format!("{}\n\n{}", a.text, a.comment) };
                            ui.ctx().copy_text(txt);
                        }
                        ui.add_space(6.0);
                        if matches!(a.kind, Kind::Highlight | Kind::Underline | Kind::Strike) {
                            for (icon, k, tip) in [(Icon::Strike, Kind::Strike, "취소선"), (Icon::Underline, Kind::Underline, "밑줄"), (Icon::Highlight, Kind::Highlight, "형광펜")] {
                                if icons::button(ui, icon, a.kind == k, ACCENT, tip).clicked() {
                                    a.kind = k;
                                }
                            }
                        } else {
                            ui.label(RichText::new(a.kind.label()).family(icons::bold()));
                        }
                        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                            if icons::button(ui, Icon::Close, false, ACCENT, "닫기").clicked() {
                                close = true;
                            }
                            if icons::button(ui, Icon::Trash, false, ACCENT, "주석 삭제").clicked() {
                                delete = true;
                            }
                        });
                    });
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        for i in 0..8 {
                            if color_dot(ui, i, a.color == i).clicked() {
                                a.color = i;
                            }
                        }
                    });
                    if !a.text.is_empty() {
                        ui.add_space(2.0);
                        ui.label(RichText::new(truncate(&a.text, 160)).italics().weak().size(12.5));
                    }
                    ui.add_space(4.0);
                    let r = ui.add(
                        egui::TextEdit::multiline(&mut a.comment)
                            .hint_text("메모를 남기세요  ·  #태그 는 Obsidian 태그가 돼요")
                            .desired_rows(5)
                            .desired_width(f32::INFINITY)
                            .margin(Margin::same(8)),
                    );
                    if a.comment.is_empty() && a.kind == Kind::Note && !r.has_focus() {
                        r.request_focus();
                    }
                    ui.add_space(2.0);
                    ui.horizontal(|ui| {
                        let who = if a.author.is_empty() { user.clone() } else { a.author.clone() };
                        ui.label(RichText::new(if who.is_empty() { "—".into() } else { who }).size(11.0).weak());
                        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                            ui.label(RichText::new(&a.date).size(11.0).weak());
                        });
                    });
                });
            });
        if delete {
            t.notes.annotations.retain(|x| x.id != id);
            t.editing = None;
            changed(t, screenshot);
            return;
        }
        if (a.kind, a.color, a.comment.clone()) != before {
            if let Some(slot) = t.notes.annotations.iter_mut().find(|x| x.id == id) {
                *slot = a;
            }
            changed(t, screenshot);
        }
        if close || ctx.input(|i| i.key_pressed(Key::Escape)) {
            t.editing = None;
        }
    }

    // ------------------------------------------------------------ notes panel

    fn notes_panel(&mut self, ui: &mut egui::Ui) {
        let pal = theme::pal(ui.ctx());
        let screenshot = self.screenshot_mode;
        let settings = self.settings.clone();
        let template = settings.obsidian.name_template.clone();

        ui.add_space(12.0);
        let mut toggle_md = false;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 14.0;
            for (is_ai, name) in [(false, "노트"), (true, "AI")] {
                let sel = self.panel_ai == is_ai;
                let r = ui.add(egui::Label::new(RichText::new(name).family(icons::bold()).size(15.0).color(if sel { pal.text } else { pal.weak })).sense(Sense::click()));
                if sel {
                    let y = r.rect.bottom() + 3.0;
                    ui.painter().hline(r.rect.left()..=r.rect.right(), y, Stroke::new(2.0, pal.accent));
                }
                if r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    self.panel_ai = is_ai;
                }
            }
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                if !self.panel_ai && self.tab_ref().is_some() && icons::small(ui, Icon::Code, self.notes_source, ACCENT, "Markdown으로 보기").clicked() {
                    toggle_md = true;
                }
            });
        });
        if toggle_md {
            self.notes_source = !self.notes_source;
        }
        ui.add_space(10.0);
        if self.panel_ai {
            self.ai_panel(ui);
            return;
        }

        let edit_id = Id::new(("note-name-edit", self.tab_ref().map(|t| t.path.clone())));
        let mut name_edit: Option<String> = ui.ctx().data(|d| d.get_temp(edit_id));
        let mut name_commit: Option<String> = None;
        let mut name_auto = false;
        let mut sync_now: Option<bool> = None;
        let mut open_settings = false;
        let notes_source = self.notes_source;

        let Some(t) = self.tab() else {
            ui.add_space(18.0);
            ui.label(RichText::new("PDF를 열면 하이라이트와 메모가 여기에 정리돼요.").size(13.0).color(pal.weak));
            return;
        };

        if notes_source {
            let mut md = t.markdown(&settings);
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.add(egui::TextEdit::multiline(&mut md).font(egui::TextStyle::Monospace).desired_width(f32::INFINITY).frame(egui::Frame::NONE));
            });
            return;
        }

        let f = &settings.format;
        let mut jump: Option<(usize, u64)> = None;
        let mut cite_fetch = false;
        let mut memo_changed = false;
        egui::ScrollArea::vertical().id_salt(("notes", t.path.clone())).auto_shrink([false, false]).show(ui, |ui| {
            // The document.
            let title = if t.notes.title.trim().is_empty() {
                t.path.file_stem().map(|s| crate::pdf::nfc(&s.to_string_lossy())).unwrap_or_default()
            } else {
                t.notes.title.trim().to_string()
            };
            ui.label(RichText::new(title).family(icons::bold()).size(16.0));
            let sub: Vec<String> = [&t.notes.author, &t.notes.institution, &t.notes.year].iter().filter(|s| !s.trim().is_empty()).map(|s| s.trim().to_string()).collect();
            if !sub.is_empty() {
                ui.add_space(2.0);
                ui.label(RichText::new(sub.join(" · ")).size(12.0).color(pal.weak));
            }

            // Obsidian: where the note goes, its name, how it's doing.
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let (r, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
                icons::paint(ui.painter(), r, Icon::Gem, pal.accent, pal.side);
                let o = &settings.obsidian;
                let place = if o.vault.is_empty() { "보관함을 고르세요".to_string() } else { format!("{} › {}", obsidian::vault_name(&o.vault), o.notes_folder) };
                let r = ui.add(egui::Label::new(RichText::new(place).size(12.0).color(pal.weak)).sense(Sense::click()).truncate());
                if r.on_hover_text("Obsidian 설정").clicked() {
                    open_settings = true;
                }
                ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                    if icons::small(ui, Icon::External, false, ACCENT, "Obsidian에서 열기").clicked() {
                        sync_now = Some(true);
                    }
                    if icons::small(ui, Icon::Sync, false, ACCENT, "지금 동기화").clicked() {
                        sync_now = Some(false);
                    }
                });
            });
            let shown = obsidian::note_title(&t.notes);
            let mut v = name_edit.clone().unwrap_or_else(|| shown.clone());
            let r = ui.add(
                egui::TextEdit::singleline(&mut v)
                    .font(FontId::new(13.5, icons::bold()))
                    .desired_width(f32::INFINITY)
                    .margin(Margin::symmetric(6, 4))
                    .background_color(Color32::TRANSPARENT)
                    .frame(egui::Frame::new().corner_radius(CornerRadius::same(6))),
            );
            let r = r.on_hover_text("Obsidian 노트 이름 — 눌러서 바꾸세요");
            if r.hovered() || r.has_focus() {
                ui.painter().rect_stroke(r.rect, 6.0, Stroke::new(1.0, if r.has_focus() { pal.accent } else { pal.border }), egui::StrokeKind::Inside);
            }
            if r.changed() {
                name_edit = Some(v.clone());
            }
            if r.lost_focus() {
                name_commit = Some(v.clone());
            }
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                match &t.sync_status {
                    Some((true, msg)) => {
                        let (d, _) = ui.allocate_exact_size(Vec2::splat(8.0), Sense::hover());
                        ui.painter().circle_filled(d.center(), 3.0, pal.good);
                        let when = msg.rsplit(" · ").next().unwrap_or("");
                        ui.label(RichText::new(format!("동기화됨 {when}")).size(11.0).color(pal.weak)).on_hover_text(msg);
                    }
                    Some((false, msg)) => {
                        ui.add(egui::Label::new(RichText::new(msg).size(11.0).color(pal.bad)).wrap());
                    }
                    None => {
                        let txt = if settings.obsidian.auto_sync { "바꾸면 자동으로 동기화" } else { "자동 동기화 꺼짐" };
                        ui.label(RichText::new(txt).size(11.0).color(pal.weak));
                    }
                }
                if t.notes.note_name_custom {
                    ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                        if link_label(ui, "자동 이름").on_hover_text("설정의 형식으로 다시 만들기").clicked() {
                            name_auto = true;
                        }
                    });
                }
            });

            // My memo.
            divider(ui, &pal);
            section_title(ui, "메모", None, &pal);
            memo_card(ui, t, &settings, &mut memo_changed);

            // Highlights.
            divider(ui, &pal);
            let n = t.notes.annotations.len();
            section_title(ui, "하이라이트", Some(n), &pal);
            if n == 0 {
                ui.label(RichText::new("형광펜으로 표시한 문장이 여기에 모여요.").size(12.5).color(pal.weak));
            }
            let sorted = t.notes.sorted();
            let groups: Vec<(Option<usize>, Vec<&Annotation>)> = if f.sort_by_color {
                (0..8).map(|c| (Some(c), sorted.iter().copied().filter(|a| a.color == c).collect::<Vec<_>>())).filter(|(_, v)| !v.is_empty()).collect()
            } else {
                vec![(None, sorted)]
            };
            for (c, list) in groups {
                if let Some(c) = c {
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        paint_dot(ui, c);
                        ui.label(RichText::new(f.categories.get(c).cloned().unwrap_or_default()).family(icons::bold()).size(12.5));
                    });
                }
                let mut last_page = None;
                for a in list {
                    if last_page != Some(a.page) {
                        ui.add_space(6.0);
                        let r = ui.add(egui::Label::new(RichText::new(format!("p. {}", t.pdf.page_label(a.page))).size(11.0).color(pal.weak)).sense(Sense::click()));
                        if r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                            jump = Some((a.page, 0));
                        }
                        last_page = Some(a.page);
                    }
                    let selected = t.editing.map(|(id, _)| id) == Some(a.id);
                    let id = Id::new(("note", a.id));
                    let hovered = ui.ctx().read_response(id).map(|r| r.hovered()).unwrap_or(false);
                    let fill = if selected { pal.accent_soft } else if hovered { ui.visuals().faint_bg_color } else { Color32::TRANSPARENT };
                    let resp = egui::Frame::new()
                        .inner_margin(Margin { left: 12, right: 6, top: 5, bottom: 5 })
                        .fill(fill)
                        .corner_radius(CornerRadius::same(6))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            match a.kind {
                                Kind::Highlight | Kind::Underline | Kind::Strike => {
                                    let mut rt = RichText::new(a.text.trim()).size(13.0).color(pal.text);
                                    rt = match a.kind {
                                        Kind::Underline => rt.underline(),
                                        Kind::Strike => rt.strikethrough(),
                                        _ => rt,
                                    };
                                    ui.label(rt);
                                }
                                Kind::Image => {
                                    ui.label(RichText::new(if a.text.is_empty() { "이미지".to_string() } else { format!("이미지 · {}", truncate(&a.text, 60)) }).size(12.5).color(pal.weak));
                                }
                                Kind::Note => {}
                            }
                            if !a.comment.trim().is_empty() {
                                ui.label(RichText::new(a.comment.trim()).size(12.5).color(pal.weak));
                            }
                        })
                        .response;
                    let bar = Rect::from_min_size(resp.rect.left_top() + Vec2::new(3.0, 6.0), Vec2::new(3.0, (resp.rect.height() - 12.0).max(4.0)));
                    ui.painter().rect_filled(bar, 1.5, color32(a.color));
                    let resp = ui.interact(resp.rect, id, Sense::click());
                    if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        jump = Some((a.page, a.id));
                    }
                    ui.add_space(1.0);
                }
            }

            // How to cite.
            divider(ui, &pal);
            cite_card(ui, t, &settings, &mut cite_fetch);
            ui.add_space(24.0);
        });
        if let Some((p, id)) = jump {
            t.scroll_to_page = Some(p);
            if id != 0 {
                let pos = ui.ctx().content_rect().center();
                t.editing = Some((id, pos));
            }
        }
        if memo_changed {
            changed(t, screenshot);
        }
        if cite_fetch {
            if t.notes.doi.trim().is_empty() {
                t.cite_status = Some("DOI가 없어요. 문서 정보에서 넣어 주세요.".into());
            } else {
                t.cite_rx = Some(start_cite_fetch(&t.notes.doi, ui.ctx()));
                t.cite_status = Some("찾는 중…".into());
            }
        }
        let committed = name_commit.is_some();
        if let Some(v) = name_commit {
            let v = v.trim().to_string();
            if !v.is_empty() && v != t.notes.note_name {
                t.notes.note_name = v;
                t.notes.note_name_custom = true;
                changed(t, screenshot);
            }
        }
        if name_auto {
            t.notes.note_name_custom = false;
            t.notes.note_name = auto_name(&t.notes, &template);
            changed(t, screenshot);
        }
        ui.ctx().data_mut(|d| match &name_edit {
            Some(v) if !committed => {
                d.insert_temp(edit_id, v.clone());
            }
            _ => d.remove::<String>(edit_id),
        });
        if let Some(open) = sync_now {
            self.sync_active(open);
        }
        if open_settings {
            self.show_settings = true;
            self.settings_tab = 1;
        }
    }

    /// Summaries and questions about the open document, with the AI the
    /// person chose (their own subscription where possible).
    fn ai_panel(&mut self, ui: &mut egui::Ui) {
        let pal = theme::pal(ui.ctx());
        let provider = self.settings.ai.provider;
        let ai_settings = self.settings.ai.clone();
        let mut open_settings = false;
        let mut to_memo: Option<String> = None;
        let screenshot = self.screenshot_mode;
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
            ui.painter().circle_filled(r.center(), 3.5, if provider.is_some() { pal.good } else { pal.weak });
            let name = provider.map(|p| p.name()).unwrap_or("AI를 고르세요");
            ui.label(RichText::new(name).size(12.0).color(pal.weak));
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                if link_label(ui, "바꾸기").clicked() {
                    open_settings = true;
                }
            });
        });
        let Some(t) = self.tab() else {
            ui.add_space(12.0);
            ui.label(RichText::new("PDF를 열면 요약하거나 내용에 대해 물어볼 수 있어요.").size(12.5).color(pal.weak));
            if open_settings {
                self.show_settings = true;
                self.settings_tab = 6;
            }
            return;
        };
        let busy = t.ai_rx.is_some();
        let mut send: Option<String> = None;
        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
            for (label, prompt) in ai::QUICK {
                let b = egui::Button::new(RichText::new(label).size(12.5)).fill(ui.visuals().faint_bg_color).corner_radius(CornerRadius::same(14)).min_size(Vec2::new(0.0, 26.0));
                if ui.add_enabled(!busy && provider.is_some(), b).clicked() {
                    send = Some(prompt.to_string());
                }
            }
        });
        ui.add_space(8.0);

        // Input at the bottom; the conversation fills the rest.
        let input_h = 92.0;
        let avail = ui.available_rect_before_wrap();
        let conv_rect = Rect::from_min_max(avail.min, Pos2::new(avail.max.x, avail.max.y - input_h));
        let input_rect = Rect::from_min_max(Pos2::new(avail.min.x, avail.max.y - input_h + 8.0), avail.max);
        ui.scope_builder(egui::UiBuilder::new().max_rect(conv_rect), |ui| {
            egui::ScrollArea::vertical().id_salt(("ai", t.path.clone())).stick_to_bottom(true).auto_shrink([false, false]).show(ui, |ui| {
                if t.ai_history.is_empty() && !busy {
                    ui.add_space(8.0);
                    ui.label(RichText::new("이 문서에 대해 무엇이든 물어보세요.\n본문·하이라이트·메모를 함께 읽고 답해요.").size(12.5).color(pal.weak));
                }
                for (i, m) in t.ai_history.iter().enumerate() {
                    ui.add_space(6.0);
                    if m.from_ai {
                        ui.label(RichText::new(t_ai_name(provider)).size(11.0).color(pal.weak));
                        render_md(ui, &m.text, &pal);
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 12.0;
                            if link_label(ui, "메모에 넣기").on_hover_text("내 메모 끝에 붙이고 Obsidian에도 남겨요").clicked() {
                                to_memo = Some(m.text.clone());
                            }
                            if link_label(ui, "복사").clicked() {
                                ui.ctx().copy_text(m.text.clone());
                            }
                        });
                    } else {
                        ui.with_layout(egui::Layout::right_to_left(Align::Min), |ui| {
                            egui::Frame::new().fill(pal.accent_soft).corner_radius(CornerRadius::same(10)).inner_margin(Margin::symmetric(10, 6)).show(ui, |ui| {
                                ui.set_max_width(ui.available_width() * 0.85);
                                let shown = ai::QUICK.iter().find(|(_, p)| *p == m.text).map(|(l, _)| l.to_string()).unwrap_or_else(|| m.text.clone());
                                ui.label(RichText::new(shown).size(13.0));
                            });
                        });
                    }
                    let _ = i;
                }
                if busy {
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new().size(14.0));
                        ui.label(RichText::new("읽고 있어요…").size(12.0).color(pal.weak));
                    });
                }
                if let Some(e) = &t.ai_error {
                    ui.add_space(6.0);
                    ui.add(egui::Label::new(RichText::new(e).size(12.0).color(pal.bad)).wrap());
                }
            });
        });
        ui.scope_builder(egui::UiBuilder::new().max_rect(input_rect), |ui| {
            let r = ui.add(
                egui::TextEdit::multiline(&mut t.ai_input)
                    .hint_text(if provider.is_some() { "질문하기…  (Enter 보내기 · Shift+Enter 줄바꿈)" } else { "먼저 AI를 고르세요" })
                    .desired_rows(2)
                    .desired_width(f32::INFINITY)
                    .margin(Margin::same(10))
                    .background_color(ui.visuals().faint_bg_color)
                    .frame(egui::Frame::new().corner_radius(CornerRadius::same(10)))
                    .interactive(provider.is_some()),
            );
            let enter = r.has_focus() && ui.input(|i| i.key_pressed(Key::Enter) && !i.modifiers.shift);
            if enter && !busy {
                let q = t.ai_input.trim().to_string();
                if !q.is_empty() {
                    send = Some(q);
                }
                t.ai_input.clear();
            }
            ui.horizontal(|ui| {
                ui.label(RichText::new(match provider { Some(p) => format!("PDF 내용이 {}로 보내져요", p.short()), None => String::new() }).size(10.5).color(pal.weak));
                ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                    if !t.ai_history.is_empty() && !busy && link_label(ui, "새 대화").clicked() {
                        t.ai_history.clear();
                        t.ai_error = None;
                    }
                });
            });
        });

        if let (Some(q), Some(p)) = (send, provider) {
            t.ai_history.push(ai::Message { from_ai: false, text: q });
            t.ai_error = None;
            let limit = if p == ai::Provider::Ollama { 24_000 } else { 150_000 };
            let context = ai_context(t, limit);
            let history = t.ai_history.clone();
            let (tx, rx) = std::sync::mpsc::channel();
            let ctx2 = ui.ctx().clone();
            std::thread::spawn(move || {
                let _ = tx.send(ai::ask(p, &ai_settings, &context, &history));
                ctx2.request_repaint();
            });
            t.ai_rx = Some(rx);
        }
        if let Some(text) = to_memo {
            append_line(&mut t.notes.memo, &format!("\n{}", text.trim()));
            changed(t, screenshot);
            self.status = "메모에 넣었어요".into();
        }
        if open_settings {
            self.show_settings = true;
            self.settings_tab = 6;
        }
    }

    // ---------------------------------------------------------------- windows

    fn meta_window(&mut self, ctx: &egui::Context) {
        if !self.show_meta {
            return;
        }
        let screenshot = self.screenshot_mode;
        let template = self.settings.obsidian.name_template.clone();
        let mut open = true;
        let mut changed_any = false;
        let mut regenerate = false;
        if let Some(t) = self.tab() {
            let auto_before = auto_name(&t.notes, &template);
            let mut fetch = false;
            egui::Window::new("문서 정보").open(&mut open).resizable(false).collapsible(false).max_width(ctx.content_rect().width() - 24.0).show(ctx, |ui| {
                egui::Grid::new("meta").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                    for (label, value, hint) in [
                        ("제목", &mut t.notes.title, ""),
                        ("저자", &mut t.notes.author, "홍길동, 김철수"),
                        ("기관", &mut t.notes.institution, "서울대학교"),
                        ("연도", &mut t.notes.year, "2024"),
                        ("DOI", &mut t.notes.doi, "10.xxxx/..."),
                        ("학술지", &mut t.notes.journal, ""),
                        ("권(호)", &mut t.notes.volume, "12"),
                        ("호", &mut t.notes.issue, "3"),
                        ("쪽", &mut t.notes.pages, "45-67"),
                        ("출판사", &mut t.notes.publisher, ""),
                    ] {
                        ui.label(label);
                        changed_any |= ui.add(egui::TextEdit::singleline(value).hint_text(hint).desired_width(340.0f32.min(ui.ctx().content_rect().width() - 140.0))).changed();
                        ui.end_row();
                    }
                    ui.label(RichText::new("노트 이름").family(icons::bold()));
                    ui.horizontal(|ui| {
                        if ui.add(egui::TextEdit::singleline(&mut t.notes.note_name).desired_width(250.0)).changed() {
                            changed_any = true;
                            t.notes.note_name_custom = true;
                        }
                        if ui.button("자동으로").on_hover_text("설정의 노트 이름 형식으로 다시 만들기").clicked() {
                            regenerate = true;
                        }
                    });
                    ui.end_row();
                    ui.label("파일");
                    ui.add(egui::Label::new(RichText::new(t.path.to_string_lossy()).size(11.0).weak()).truncate());
                    ui.end_row();
                });
                ui.add_space(4.0);
                ui.label(RichText::new("노트 이름이 Obsidian 노트의 파일 이름이 돼요. 이름을 바꾸면 기존 노트도 함께 옮겨져요.").size(11.5).weak());
                ui.horizontal(|ui| {
                    if ui.button("PDF에서 다시 찾기").on_hover_text("첫 페이지에서 저자·기관·연도·DOI를 다시 찾습니다").clicked() {
                        t.notes.author = t.pdf.author().unwrap_or_default();
                        t.notes.institution.clear();
                        t.notes.year.clear();
                        t.notes.note_name_custom = false;
                        fill_details(&t.pdf, &mut t.notes, &template);
                        changed_any = true;
                    }
                    if ui.add_enabled(!t.notes.doi.trim().is_empty(), egui::Button::new("DOI로 인용 정보 가져오기")).on_hover_text("CrossRef에서 저자·학술지·권·호·쪽을 가져옵니다 (DOI만 보냅니다)").clicked() {
                        fetch = true;
                    }
                });
                if let Some(st) = &t.cite_status {
                    ui.label(RichText::new(st).size(11.5).weak());
                }
            });
            if fetch {
                t.cite_rx = Some(start_cite_fetch(&t.notes.doi, ctx));
                t.cite_status = Some("CrossRef에서 찾는 중…".into());
            }
            if changed_any && !regenerate && !t.notes.note_name_custom && t.notes.note_name == auto_before {
                regenerate = true;
            }
            if regenerate {
                t.notes.note_name_custom = false;
                t.notes.note_name = auto_name(&t.notes, &template);
                changed_any = true;
            }
            if changed_any {
                changed(t, screenshot);
            }
        }
        self.show_meta = open;
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        if !self.show_settings {
            return;
        }
        let mut open = true;
        let mut changed_any = false;
        let mut dark_change: Option<bool> = None;
        let mut pdf_dark_changed = false;
        let mut apply_template_all = false;
        let preview_notes = self.tab_ref().map(|t| t.notes.clone());
        egui::Window::new("설정").open(&mut open).default_width(500.0f32.min(ctx.content_rect().width() - 24.0)).max_width(ctx.content_rect().width() - 24.0).collapsible(false).vscroll(true).show(ctx, |ui| {
            ui.horizontal(|ui| {
                for (i, name) in ["노트 형식", "Obsidian", "인용", "단축키", "색 분류", "화면 · 일반", "AI"].iter().enumerate() {
                    let sel = self.settings_tab == i as u8;
                    let b = egui::Button::new(RichText::new(*name).color(if sel { Color32::WHITE } else { ui.visuals().text_color() }))
                        .fill(if sel { ACCENT } else { Color32::TRANSPARENT })
                        .corner_radius(CornerRadius::same(8));
                    if ui.add(b).clicked() {
                        self.settings_tab = i as u8;
                    }
                }
            });
            ui.separator();
            let s = &mut self.settings;
            match self.settings_tab {
                0 => {
                    let f = &mut s.format;
                    section(ui, "노트에 넣을 것");
                    changed_any |= ui.checkbox(&mut f.use_page_links, "페이지 링크 (노트에서 PDF 해당 쪽으로 이동)").changed();
                    changed_any |= ui.checkbox(&mut f.include_page, "주석마다 쪽 번호").changed();
                    changed_any |= ui.checkbox(&mut f.include_category, "색 분류 이름").changed();
                    changed_any |= ui.checkbox(&mut f.include_color, "색 이름").changed();
                    changed_any |= ui.checkbox(&mut f.include_type, "주석 종류").changed();
                    changed_any |= ui.checkbox(&mut f.include_author, "작성자").changed();
                    changed_any |= ui.checkbox(&mut f.include_date, "날짜").changed();
                    changed_any |= ui.checkbox(&mut f.include_spacer, "주석 사이 구분선 (***)").changed();
                    section(ui, "정리");
                    changed_any |= ui.checkbox(&mut f.sort_by_color, "색 분류별로 묶기").changed();
                    changed_any |= ui.checkbox(&mut f.use_atx_header, "# 제목 형식 머리글").changed();
                    changed_any |= ui.checkbox(&mut f.use_blockquotes, "인용(>)으로 표시").changed();
                    changed_any |= ui.checkbox(&mut f.lower_case_tags, "태그를 소문자로").changed();
                    ui.horizontal(|ui| {
                        changed_any |= ui.checkbox(&mut f.use_text_wrapping, "줄바꿈 폭").changed();
                        changed_any |= ui.add(egui::DragValue::new(&mut f.page_width).range(30..=200)).changed();
                    });
                    section(ui, "앞뒤 기호");
                    egui::Grid::new("tags").num_columns(3).show(ui, |ui| {
                        for (name, a, b) in [
                            ("형광펜", &mut f.highlight_start, &mut f.highlight_end),
                            ("밑줄", &mut f.underline_start, &mut f.underline_end),
                            ("취소선", &mut f.strike_start, &mut f.strike_end),
                        ] {
                            ui.label(name);
                            changed_any |= ui.add(egui::TextEdit::singleline(a).desired_width(90.0)).changed();
                            changed_any |= ui.add(egui::TextEdit::singleline(b).desired_width(90.0)).changed();
                            ui.end_row();
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("내 이름 (작성자)");
                        changed_any |= ui.text_edit_singleline(&mut f.user_name).changed();
                    });
                }
                1 => {
                    let o = &mut s.obsidian;
                    section(ui, "보관함 (vault)");
                    ui.horizontal(|ui| {
                        egui::ComboBox::from_id_salt("vault")
                            .width(300.0)
                            .selected_text(if o.vault.is_empty() { "고르세요".to_string() } else { obsidian::vault_name(&o.vault) })
                            .show_ui(ui, |ui| {
                                for v in &self.vaults {
                                    if ui.selectable_label(o.vault == *v, obsidian::vault_name(v)).on_hover_text(v).clicked() {
                                        o.vault = v.clone();
                                        changed_any = true;
                                    }
                                }
                            });
                        if ui.button("폴더 고르기…").clicked() {
                            if let Some(p) = rfd::FileDialog::new().pick_folder() {
                                o.vault = p.to_string_lossy().to_string();
                                changed_any = true;
                            }
                        }
                    });
                    if !o.vault.is_empty() {
                        ui.label(RichText::new(&o.vault).size(11.0).weak());
                        if !Path::new(&o.vault).join(".obsidian").is_dir() {
                            ui.label(RichText::new("이 폴더에는 .obsidian 폴더가 없어요. Obsidian 보관함이 맞는지 확인하세요.").size(11.5).color(theme::pal(ui.ctx()).bad));
                        }
                    }
                    section(ui, "노트 이름 (제목)");
                    ui.horizontal(|ui| {
                        changed_any |= ui.add(egui::TextEdit::singleline(&mut o.name_template).desired_width(260.0)).changed();
                        if ui.small_button("기본").clicked() {
                            o.name_template = naming::DEFAULT_TEMPLATE.into();
                            changed_any = true;
                        }
                    });
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new("넣을 수 있는 것:").size(11.5).weak());
                        for tok in ["{저자}", "{저자전체}", "{기관}", "{연도}", "{제목}", "{파일}"] {
                            if ui.small_button(tok).on_hover_text("누르면 끝에 붙여요").clicked() {
                                o.name_template.push_str(tok);
                                changed_any = true;
                            }
                        }
                    });
                    if let Some(n) = &preview_notes {
                        ui.label(RichText::new(format!("예: {}", auto_name(n, &o.name_template))).size(12.0));
                    }
                    if ui.button("열린 문서의 노트 이름을 이 형식으로 다시 만들기").on_hover_text("직접 바꾼 이름도 바뀝니다").clicked() {
                        apply_template_all = true;
                    }
                    changed_any |= ui.checkbox(&mut o.heading_is_note_name, "노트 안 첫 제목(#)도 노트 이름으로").changed();
                    ui.add_space(6.0);
                    egui::Grid::new("obs").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                        ui.label("노트 폴더");
                        changed_any |= ui.text_edit_singleline(&mut o.notes_folder).changed();
                        ui.end_row();
                        ui.label("첨부 폴더");
                        changed_any |= ui.text_edit_singleline(&mut o.attachments_folder).changed();
                        ui.end_row();
                        ui.label("기본 태그");
                        changed_any |= ui.text_edit_singleline(&mut o.extra_tags).changed();
                        ui.end_row();
                    });
                    section(ui, "동기화");
                    changed_any |= ui.checkbox(&mut o.auto_sync, "주석을 바꾸면 자동으로 동기화").changed();
                    changed_any |= ui.checkbox(&mut o.copy_pdf, "PDF를 첨부 폴더에 복사 (Obsidian 안에서 쪽 링크가 열려요)").changed();
                    changed_any |= ui.checkbox(&mut o.frontmatter, "속성(YAML) 넣기: 제목·저자·기관·연도·출처·태그·날짜").changed();
                    changed_any |= ui.checkbox(&mut o.callouts, "인용을 Obsidian 콜아웃(> [!quote])으로").changed();
                    ui.horizontal(|ui| {
                        ui.label("쪽 링크");
                        changed_any |= ui.radio_value(&mut o.link_style, LinkStyle::ObsidianPdf, "[[문서.pdf#page=3]]").changed();
                        changed_any |= ui.radio_value(&mut o.link_style, LinkStyle::FileLink, "파일 링크").changed();
                        changed_any |= ui.radio_value(&mut o.link_style, LinkStyle::None, "없음").changed();
                    });
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new("노트 이름은 PDF를 열 때 ‘저자, 기관 (연도)’로 자동으로 정해져요. 노트 안의 ‘%% highlights:start %%’ ~ ‘%% highlights:end %%’ 구역만 자동으로 바뀌고, 그 밖에 직접 쓴 내용과 추가한 태그는 그대로 남아요.")
                            .size(11.5)
                            .weak(),
                    );
                }
                2 => {
                    let c = &mut s.format.citation;
                    section(ui, "노트 맨 뒤의 ‘인용’ 부분");
                    changed_any |= ui.checkbox(&mut c.enabled, "노트 끝에 인용 방법 넣기").changed();
                    ui.add_enabled_ui(c.enabled, |ui| {
                        changed_any |= ui.checkbox(&mut c.in_text, "본문 인용 — (Doe et al., 2021)").changed();
                        changed_any |= ui.checkbox(&mut c.apa, "APA 7판").changed();
                        changed_any |= ui.checkbox(&mut c.mla, "MLA 9판").changed();
                        changed_any |= ui.checkbox(&mut c.chicago, "Chicago (저자-연도)").changed();
                        changed_any |= ui.checkbox(&mut c.bibtex, "BibTeX (Zotero·LaTeX용)").changed();
                    });
                    section(ui, "정보 채우기");
                    changed_any |= ui.checkbox(&mut c.auto_fetch, "PDF를 열 때 DOI로 CrossRef에서 학술지·권·호·쪽 가져오기").changed();
                    ui.label(RichText::new("CrossRef에는 DOI만 보내요. 가져온 내용은 문서 정보(ⓘ)에서 고칠 수 있어요.").size(11.5).weak());
                }
                3 => {
                    section(ui, "단축키");
                    ui.label(RichText::new("‘바꾸기’를 누르고 원하는 키를 누르세요. Esc는 취소, ⌫는 지우기. 한글 입력 상태에서도 같은 자리의 키로 동작해요.").size(11.5).weak());
                    ui.add_space(4.0);
                    egui::Grid::new("keys").num_columns(3).spacing([14.0, 6.0]).striped(true).show(ui, |ui| {
                        for a in Action::ALL {
                            ui.label(a.label());
                            let rec = self.recording == Some(a);
                            let text = if rec { "키를 누르세요…".to_string() } else { s.keymap.get(a).map(|b| b.label()).unwrap_or_else(|| "없음".into()) };
                            let b = egui::Button::new(RichText::new(text).family(icons::bold()).color(if rec { Color32::WHITE } else { ui.visuals().text_color() }))
                                .fill(if rec { ACCENT } else { ui.visuals().faint_bg_color })
                                .min_size(Vec2::new(130.0, 24.0));
                            if ui.add(b).on_hover_text("바꾸기").clicked() {
                                self.recording = if rec { None } else { Some(a) };
                            }
                            if ui.small_button("기본값").clicked() {
                                let d = a.default_binding();
                                s.keymap.set(a, d);
                                changed_any = true;
                            }
                            ui.end_row();
                        }
                    });
                    ui.add_space(4.0);
                    if ui.button("모두 기본값으로").clicked() {
                        s.keymap.reset();
                        changed_any = true;
                    }
                    ui.label(RichText::new(crate::shortcuts::plat("고정 단축키: ⌘Z 되돌리기 · ⌘⇧Z 다시 하기 · ⌘O 열기 · ⌘S 저장 · ⌘W 닫기 · ⌘+ / ⌘− / ⌘0 확대 · ←/→ 페이지 · Ctrl+Tab 다음 문서")).size(11.5).weak());
                }
                4 => {
                    ui.label("색마다 분류 이름을 정하세요 (색 분류별로 묶을 때 제목이 돼요).");
                    let f = &mut s.format;
                    while f.categories.len() < 8 {
                        f.categories.push(String::new());
                    }
                    egui::Grid::new("cats").num_columns(2).show(ui, |ui| {
                        for i in 0..8 {
                            paint_dot(ui, i);
                            changed_any |= ui.text_edit_singleline(&mut f.categories[i]).changed();
                            ui.end_row();
                        }
                    });
                }
                6 => {
                    if self.ai_status.is_empty() {
                        self.ai_status = ai::Provider::ALL.iter().map(|p| (*p, ai::check(*p))).collect();
                    }
                    section(ui, "어떤 AI로 요약하고 대화할까요");
                    for (p, st) in self.ai_status.clone() {
                        ui.horizontal(|ui| {
                            if ui.radio(s.ai.provider == Some(p), p.name()).clicked() {
                                s.ai.provider = Some(p);
                                changed_any = true;
                            }
                            match &st {
                                Ok(d) => ui.label(RichText::new(format!("· {d}")).size(11.5).color(theme::pal(ui.ctx()).good)),
                                Err(e) => ui.label(RichText::new(format!("· {e}")).size(11.5).color(theme::pal(ui.ctx()).weak)),
                            };
                        });
                    }
                    if let Some(p) = s.ai.provider {
                        ui.add_space(4.0);
                        ui.label(RichText::new(p.how()).size(11.5).weak());
                        match p {
                            ai::Provider::Ollama => {
                                ui.horizontal(|ui| {
                                    ui.label("모델");
                                    changed_any |= ui.add(egui::TextEdit::singleline(&mut s.ai.ollama_model).hint_text("비우면 첫 번째 모델").desired_width(200.0)).changed();
                                });
                            }
                            ai::Provider::AnthropicKey | ai::Provider::OpenAiKey => {
                                ui.horizontal(|ui| {
                                    ui.label("모델");
                                    let m = if p == ai::Provider::AnthropicKey { &mut s.ai.anthropic_model } else { &mut s.ai.openai_model };
                                    changed_any |= ui.add(egui::TextEdit::singleline(m).desired_width(200.0)).changed();
                                });
                                ui.horizontal(|ui| {
                                    ui.label("API 키");
                                    ui.add(egui::TextEdit::singleline(&mut self.ai_key_input).password(true).hint_text(if ai::key_get(p).is_some() { "저장됨 — 바꾸려면 새로 넣기" } else { "sk-…" }).desired_width(220.0));
                                    if ui.button(ai::KEY_SAVE_LABEL).clicked() && !self.ai_key_input.trim().is_empty() {
                                        let _ = ai::key_set(p, &self.ai_key_input);
                                        self.ai_key_input.clear();
                                        self.ai_status.clear();
                                    }
                                });
                            }
                            _ => {}
                        }
                    }
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.label("답하는 언어");
                        changed_any |= ui.add(egui::TextEdit::singleline(&mut s.ai.language).desired_width(120.0)).changed();
                        if ui.button("다시 확인").clicked() {
                            self.ai_status.clear();
                        }
                    });
                    ui.label(RichText::new("요약·질문할 때 그 문서의 본문, 하이라이트, 메모가 고른 AI로 보내져요. 다른 때에는 아무것도 보내지 않아요.").size(11.5).weak());
                }
                _ => {
                    section(ui, "화면");
                    ui.horizontal(|ui| {
                        ui.label("화면 모드");
                        let mut mode = if s.follow_system { 0 } else if s.dark { 2 } else { 1 };
                        let before = mode;
                        segmented(ui, &mut mode, &[(0, "시스템 따라가기"), (1, "라이트"), (2, "다크")]);
                        if mode != before {
                            s.follow_system = mode == 0;
                            if mode != 0 {
                                s.dark = mode == 2;
                            }
                            dark_change = Some(mode == 2);
                            changed_any = true;
                        }
                    });
                    if ui.checkbox(&mut s.dark_pdf, "어두운 화면에서 PDF 페이지도 어둡게").changed() {
                        pdf_dark_changed = true;
                        changed_any = true;
                    }
                    ui.label(RichText::new(if cfg!(target_os = "macos") { "확대·축소: 트랙패드를 두 손가락으로 벌리거나, ⌘(또는 Ctrl)을 누른 채 스크롤하세요." } else { "확대·축소: Ctrl을 누른 채 스크롤하거나, 트랙패드를 두 손가락으로 벌리세요." }).size(11.5).weak());
                    section(ui, "일반");
                    changed_any |= ui.checkbox(&mut s.save_into_pdf, "문서를 닫을 때 주석을 PDF 파일에도 저장").changed();
                    ui.horizontal(|ui| {
                        ui.label("기본 색");
                        egui::ComboBox::from_id_salt("defcolor").selected_text(COLOR_NAMES_KO[s.default_color]).show_ui(ui, |ui| {
                            for i in 0..8 {
                                if ui.selectable_label(s.default_color == i, COLOR_NAMES_KO[i]).clicked() {
                                    s.default_color = i;
                                    changed_any = true;
                                }
                            }
                        });
                    });
                }
            }
        });
        self.show_settings = open;
        if !open {
            self.recording = None;
        }
        if apply_template_all {
            let template = self.settings.obsidian.name_template.clone();
            let screenshot = self.screenshot_mode;
            for t in &mut self.tabs {
                t.notes.note_name_custom = false;
                t.notes.note_name = auto_name(&t.notes, &template);
                changed(t, screenshot);
            }
        }
        if dark_change.is_some() {
            // Follow the system or the new choice from the next frame.
            self.applied_dark = !self.want_dark(ctx);
        }
        if pdf_dark_changed {
            self.clear_textures();
        }
        if changed_any {
            if !self.screenshot_mode {
                self.settings.save();
            }
            for t in &mut self.tabs {
                t.changed_at = Some(Instant::now());
            }
        }
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        let typing = ctx.egui_wants_keyboard_input();
        // Recording a new shortcut in Settings: the next key press is it.
        if let Some(a) = self.recording {
            match shortcuts::record(ctx) {
                shortcuts::Recorded::Nothing => {}
                shortcuts::Recorded::Cancel => self.recording = None,
                shortcuts::Recorded::Clear => {
                    self.settings.keymap.set(a, None);
                    self.recording = None;
                    self.save_settings();
                }
                shortcuts::Recorded::Set(b) => {
                    self.settings.keymap.set(a, Some(b));
                    self.recording = None;
                    self.save_settings();
                }
            }
            ctx.input_mut(|i| i.events.retain(|e| !matches!(e, egui::Event::Key { .. } | egui::Event::Text(_))));
            return;
        }
        for a in self.settings.keymap.pressed(ctx, typing) {
            self.run_action(a, ctx);
        }
        let (undo, redo) = ctx.input(|i| {
            let mut u = (false, false);
            for e in &i.events {
                if let egui::Event::Key { key, physical_key, pressed: true, modifiers, .. } = e {
                    if physical_key.unwrap_or(*key) == Key::Z && modifiers.command && !typing {
                        if modifiers.shift { u.1 = true } else { u.0 = true }
                    }
                }
            }
            u
        });
        if undo || redo {
            let screenshot = self.screenshot_mode;
            let msg = self.tab().and_then(|t| undo_redo(t, redo, screenshot));
            self.status = msg.unwrap_or_else(|| if redo { "다시 할 것이 없어요.".into() } else { "되돌릴 것이 없어요.".into() });
        }
        let (open, save, zin, zout, zreset, next, prev, close, tab_next) = ctx.input(|i| {
            let cmd = i.modifiers.command && !i.modifiers.shift && !i.modifiers.alt;
            (
                cmd && i.key_pressed(Key::O),
                cmd && i.key_pressed(Key::S),
                cmd && (i.key_pressed(Key::Plus) || i.key_pressed(Key::Equals)),
                cmd && i.key_pressed(Key::Minus),
                cmd && i.key_pressed(Key::Num0),
                !typing && (i.key_pressed(Key::PageDown) || i.key_pressed(Key::ArrowRight)),
                !typing && (i.key_pressed(Key::PageUp) || i.key_pressed(Key::ArrowLeft)),
                cmd && i.key_pressed(Key::W),
                i.modifiers.ctrl && i.key_pressed(Key::Tab),
            )
        });
        if open {
            self.open_dialog();
        }
        if save {
            self.save_pdf_active();
        }
        let z = self.tab_ref().map(|t| t.zoom).unwrap_or(1.0);
        if zin {
            self.set_zoom(z * 1.15);
        }
        if zout {
            self.set_zoom(z / 1.15);
        }
        if zreset {
            self.set_zoom(1.0);
        }
        if let Some(t) = self.tab() {
            if next {
                t.scroll_to_page = Some(t.current_page + 1);
            }
            if prev && t.current_page > 0 {
                t.scroll_to_page = Some(t.current_page - 1);
            }
        }
        if close && !self.tabs.is_empty() {
            let i = self.active();
            self.close_tab(i);
        }
        if tab_next && self.tabs.len() > 1 {
            let i = (self.active() + 1) % self.tabs.len();
            self.show_tab(i);
        }
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).filter(|p| p.extension().map(|e| e.eq_ignore_ascii_case("pdf")).unwrap_or(false)).collect()
        });
        for p in dropped {
            self.open(&p);
        }
    }

    fn save_settings(&self) {
        if !self.screenshot_mode {
            self.settings.save();
        }
    }

    /// Do what a shortcut stands for.
    fn run_action(&mut self, a: Action, ctx: &egui::Context) {
        let mark = |tool: Tool| -> Tool { tool };
        match a {
            Action::Highlight => self.mark_or_tool(mark(Tool::Highlight)),
            Action::Underline => self.mark_or_tool(Tool::Underline),
            Action::Strike => self.mark_or_tool(Tool::Strike),
            Action::Note => self.mark_or_tool(Tool::Note),
            Action::Capture => {
                self.tool = if self.tool == Tool::Image { Tool::Select } else { Tool::Image };
                self.status = if self.tool == Tool::Image { "캡처: 페이지에서 영역을 끌어서 고르세요.".into() } else { "선택 도구".into() };
            }
            Action::Select => self.tool = Tool::Select,
            Action::NextDoc | Action::PrevDoc => {
                let n = self.tabs.len();
                if n > 1 {
                    let cur = self.active();
                    let i = if a == Action::NextDoc { (cur + 1) % n } else { (cur + n - 1) % n };
                    self.show_tab(i);
                    self.status = format!("{} ({}/{})", self.tabs[i].name(), i + 1, n);
                }
            }
            Action::OtherPane => {
                if self.right.is_some() {
                    self.focus_right = !self.focus_right;
                } else if self.tabs.len() > 1 {
                    let n = self.tabs.len();
                    let i = (self.active() + 1) % n;
                    self.show_tab(i);
                }
            }
            Action::ToggleSplit => self.toggle_split(),
            Action::SyncObsidian => self.sync_active(false),
            Action::OpenObsidian => self.sync_active(true),
            Action::CopyCitation => {
                if let Some(t) = self.tab_ref() {
                    let w = citation::Work::from_notes(&t.notes);
                    ctx.copy_text(w.apa().replace('*', ""));
                    self.status = "APA 인용을 복사했어요.".into();
                }
            }
            Action::ToggleDark => {
                let d = !self.applied_dark;
                self.set_dark(ctx, d);
            }
        }
    }

    /// With text selected (or an annotation open), mark it with `tool`'s kind;
    /// otherwise switch to that tool (or back to Select).
    fn mark_or_tool(&mut self, tool: Tool) {
        let color = self.color;
        let user = self.settings.format.user_name.clone();
        let screenshot = self.screenshot_mode;
        let kind = tool.kind().unwrap();
        if let Some(t) = self.tab() {
            if t.pending.is_some() {
                apply_pending(t, kind, color, &user, screenshot);
                return;
            }
            if let Some((id, _)) = t.editing {
                if kind != Kind::Note {
                    if let Some(a) = t.notes.annotations.iter_mut().find(|a| a.id == id) {
                        if matches!(a.kind, Kind::Highlight | Kind::Underline | Kind::Strike) && a.kind != kind {
                            a.kind = kind;
                            changed(t, screenshot);
                            return;
                        }
                    }
                }
            }
        }
        self.tool = if self.tool == tool { Tool::Select } else { tool };
    }

    /// Short messages fade in at the bottom of the page area and go away.
    fn toast(&mut self, ctx: &egui::Context, area: Rect) {
        if self.status != self.toast_text {
            self.toast_text = self.status.clone();
            self.toast_at = Instant::now();
        }
        if self.toast_text.is_empty() || !area.is_positive() {
            return;
        }
        let age = self.toast_at.elapsed().as_secs_f32();
        let shown = 3.2;
        if age > shown {
            return;
        }
        let alpha = (age / 0.15).min(1.0).min((shown - age) / 0.4).clamp(0.0, 1.0);
        ctx.request_repaint();
        let dark = self.applied_dark;
        egui::Area::new(Id::new("toast"))
            .order(Order::Foreground)
            .interactable(false)
            .fixed_pos(Pos2::new(area.center().x, area.bottom() - 28.0))
            .pivot(Align2::CENTER_BOTTOM)
            .show(ctx, |ui| {
                ui.set_opacity(alpha);
                egui::Frame::new()
                    .fill(if dark { Color32::from_rgb(58, 58, 66) } else { Color32::from_rgb(36, 36, 42) })
                    .corner_radius(CornerRadius::same(10))
                    .inner_margin(Margin::symmetric(14, 8))
                    .shadow(egui::epaint::Shadow { offset: [0, 4], blur: 16, spread: 0, color: Color32::from_black_alpha(50) })
                    .show(ui, |ui| {
                        ui.label(RichText::new(truncate(&self.toast_text, 80)).size(12.5).color(Color32::from_rgb(245, 245, 248)));
                    });
            });
    }

    fn is_phone(&self, ctx: &egui::Context) -> bool {
        self.force_phone || ctx.content_rect().width() < 640.0
    }

    /// The phone layout: title bar, PDF/노트 switch, the page (or notes)
    /// filling the screen, and the annotation tools along the bottom.
    fn phone_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let pal = theme::pal(&ctx);
        let has_doc = !self.tabs.is_empty();

        egui::Panel::top("p-top")
            .exact_size(52.0)
            .frame(egui::Frame::new().fill(pal.bar).inner_margin(Margin::symmetric(8, 6)).stroke(Stroke::new(1.0, pal.border)))
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    let docs = icons::button(ui, Icon::Folder, false, ACCENT, "문서");
                    egui::Popup::menu(&docs).show(|ui| {
                        ui.set_min_width(240.0);
                        let mut pick = None;
                        let active = self.active();
                        for (i, t) in self.tabs.iter().enumerate() {
                            if ui.selectable_label(i == active, truncate(&t.name(), 34)).clicked() {
                                pick = Some(i);
                            }
                        }
                        if let Some(i) = pick {
                            self.left = i;
                            self.right = None;
                            self.focus_right = false;
                        }
                        if !self.tabs.is_empty() {
                            ui.separator();
                        }
                        for p in phone_library().into_iter().take(12) {
                            let name = p.file_name().map(|s| crate::pdf::nfc(&s.to_string_lossy())).unwrap_or_default();
                            if !self.tabs.iter().any(|t| t.path == p) && ui.button(format!("＋ {}", truncate(&name, 32))).clicked() {
                                self.open(&p);
                            }
                        }
                        #[cfg(not(target_os = "ios"))]
                        if ui.button("PDF 열기…").clicked() {
                            self.open_dialog();
                        }
                        if has_doc && ui.button("이 문서 닫기").clicked() {
                            let i = self.active();
                            self.close_tab(i);
                        }
                    });
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        let w = (ui.available_width() - 80.0).max(60.0);
                        ui.set_width(w);
                        match self.tab_ref() {
                            Some(t) => {
                                ui.add(egui::Label::new(RichText::new(obsidian::note_title(&t.notes)).family(icons::bold()).size(14.0)).truncate());
                                ui.label(RichText::new(format!("{} / {}쪽 · 주석 {}", t.current_page + 1, t.pdf.page_count(), t.notes.annotations.len())).size(11.0).weak());
                            }
                            None => {
                                ui.label(RichText::new("PDF Notes").family(icons::bold()).size(15.0));
                            }
                        }
                    });
                    ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        let more = icons::button(ui, Icon::More, false, ACCENT, "더 보기");
                        egui::Popup::menu(&more).show(|ui| {
                            ui.set_min_width(220.0);
                            if has_doc {
                                if ui.button("Obsidian에서 열기").clicked() {
                                    self.sync_active(true);
                                }
                                if ui.button("Markdown 복사").clicked() {
                                    let s = self.settings.clone();
                                    if let Some(t) = self.tab() {
                                        let md = t.markdown(&s);
                                        ui.ctx().copy_text(md);
                                    }
                                }
                                if ui.button("문서 정보").clicked() {
                                    self.show_meta = true;
                                }
                                ui.separator();
                            }
                            let dark = self.applied_dark;
                            if ui.button(if dark { "밝은 화면" } else { "어두운 화면" }).clicked() {
                                self.set_dark(&ctx, !dark);
                            }
                            if ui.button("설정").clicked() {
                                self.show_settings = true;
                            }
                        });
                        ui.add_enabled_ui(has_doc, |ui| {
                            if icons::button(ui, Icon::Gem, false, ACCENT, "Obsidian 동기화").clicked() {
                                self.sync_active(false);
                            }
                        });
                    });
                });
            });

        if has_doc {
            egui::Panel::top("p-seg").exact_size(42.0).frame(egui::Frame::new().fill(pal.bar).inner_margin(Margin::symmetric(12, 6))).show(ui, |ui| {
                ui.vertical_centered_justified(|ui| {
                    segmented_wide(ui, &mut self.phone_notes, &[(false, "PDF"), (true, "노트")]);
                });
            });
        }

        if has_doc && !self.phone_notes {
            egui::Panel::bottom("p-tools")
                .exact_size(56.0)
                .frame(egui::Frame::new().fill(pal.bar).inner_margin(Margin::symmetric(8, 6)).stroke(Stroke::new(1.0, pal.border)))
                .show(ui, |ui| {
                    ui.horizontal_centered(|ui| {
                        let n = Tool::ALL.len() as f32 + 1.0;
                        let gap = ((ui.available_width() - n * 32.0) / n).clamp(2.0, 18.0);
                        ui.spacing_mut().item_spacing.x = gap;
                        ui.add_space(gap / 2.0);
                        for tl in Tool::ALL {
                            if icons::button(ui, tl.icon(), self.tool == tl, ACCENT, tl.tip()).clicked() {
                                self.tool_clicked(tl);
                            }
                        }
                        let (r, resp) = ui.allocate_exact_size(Vec2::new(32.0, 28.0), Sense::click());
                        ui.painter().circle_filled(r.center(), 9.0, color32(self.color));
                        ui.painter().circle_stroke(r.center(), 11.5, Stroke::new(1.5, pal.border));
                        egui::Popup::menu(&resp).show(|ui| {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 0.0;
                                for i in 0..8 {
                                    if color_dot(ui, i, self.color == i).clicked() {
                                        self.color = i;
                                    }
                                }
                            });
                        });
                    });
                });
        }

        egui::CentralPanel::default().frame(egui::Frame::new().fill(if self.phone_notes { pal.side } else { pal.canvas })).show(ui, |ui| {
            if !has_doc {
                self.phone_welcome(ui);
                return;
            }
            if self.phone_notes {
                egui::Frame::new().inner_margin(Margin::symmetric(14, 4)).show(ui, |ui| self.notes_panel(ui));
                return;
            }
            let vc = ViewCtx { tool: self.tool, color: self.color, dark_pages: self.dark_pages(), focused: true, touch: true };
            let user = self.settings.format.user_name.clone();
            let screenshot = self.screenshot_mode;
            let i = self.left;
            let (_, st) = page_view(ui, &mut self.tabs[i], vc, &user, screenshot, "phone");
            if let Some(st) = st {
                self.status = st;
            }
        });
    }

    fn phone_welcome(&mut self, ui: &mut egui::Ui) {
        let pal = theme::pal(ui.ctx());
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.add_space(24.0);
            ui.vertical_centered(|ui| {
                let (lr, _) = ui.allocate_exact_size(Vec2::splat(84.0), Sense::hover());
                icons::paint_logo(ui.painter(), lr, true);
                ui.label(RichText::new("PDF Notes").family(icons::bold()).size(28.0));
                ui.label(RichText::new("형광펜이 곧 Obsidian 노트가 됩니다").size(14.0).weak());
            });
            ui.add_space(20.0);
            egui::Frame::new().inner_margin(Margin::symmetric(16, 0)).show(ui, |ui| {
                theme::card(&pal).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new("내 PDF").family(icons::bold()).size(15.0));
                    let lib = phone_library();
                    if lib.is_empty() {
                        ui.label(RichText::new("파일 앱 → 나의 iPhone → PDF Notes 폴더에 PDF를 넣으면 여기에 보여요.").size(13.0).weak());
                    }
                    for p in lib.into_iter().take(30) {
                        let name = p.file_name().map(|s| crate::pdf::nfc(&s.to_string_lossy())).unwrap_or_default();
                        if ui.add(egui::Button::new(RichText::new(name).size(14.0)).fill(Color32::TRANSPARENT).min_size(Vec2::new(ui.available_width(), 34.0))).clicked() {
                            self.open(&p);
                        }
                    }
                    #[cfg(not(target_os = "ios"))]
                    if ui.button("PDF 열기…").clicked() {
                        self.open_dialog();
                    }
                });
                ui.add_space(12.0);
                theme::card(&pal).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
                        icons::paint(ui.painter(), r, Icon::Gem, pal.accent, pal.card);
                        ui.label(RichText::new("Obsidian").family(icons::bold()));
                    });
                    let o = &self.settings.obsidian;
                    ui.label(RichText::new(if o.vault.is_empty() { "설정에서 보관함을 고르세요".to_string() } else { format!("보관함 {} · {}", obsidian::vault_name(&o.vault), o.notes_folder) }).size(13.0).weak());
                });
            });
        });
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let want = self.want_dark(&ctx);
        if want != self.applied_dark {
            self.applied_dark = want;
            theme::apply(&ctx, want);
            self.clear_textures();
        }
        // AI answers.
        for t in &mut self.tabs {
            let got = t.ai_rx.as_ref().and_then(|rx| rx.try_recv().ok());
            if let Some(res) = got {
                t.ai_rx = None;
                match res {
                    Ok(text) => t.ai_history.push(ai::Message { from_ai: true, text }),
                    Err(e) => t.ai_error = Some(e),
                }
            }
        }
        // CrossRef answers.
        let template = self.settings.obsidian.name_template.clone();
        let screenshot = self.screenshot_mode;
        for t in &mut self.tabs {
            let got = t.cite_rx.as_ref().and_then(|rx| rx.try_recv().ok());
            if let Some(res) = got {
                t.cite_rx = None;
                match res {
                    Ok(found) => {
                        apply_found(&mut t.notes, found, &template);
                        t.cite_status = Some("CrossRef에서 인용 정보를 가져왔어요.".into());
                        changed(t, screenshot);
                    }
                    Err(e) => t.cite_status = Some(e),
                }
            }
        }
        // Zooming belongs to the document, not to the whole window.
        ctx.options_mut(|o| o.zoom_with_keyboard = false);
        self.shortcuts(&ctx);
        if self.is_phone(&ctx) {
            self.phone_ui(ui);
            self.pending_popover(&ctx);
            self.edit_popover(&ctx);
            self.meta_window(&ctx);
            self.settings_window(&ctx);
            self.background_work(&ctx);
            return;
        }
        let dark = self.applied_dark;
        let pal = theme::pal_for(dark);
        let (bar_fill, side_fill, line) = (pal.bar, pal.side, pal.border);

        egui::Panel::top("toolbar")
            .exact_size(54.0)
            .frame(egui::Frame::new().fill(bar_fill).inner_margin(Margin::symmetric(12, 6)).stroke(Stroke::new(1.0, line)))
            .show(ui, |ui| self.toolbar(ui));
        if self.tabs.len() >= 2 {
            egui::Panel::top("tabbar")
                .exact_size(34.0)
                .frame(egui::Frame::new().fill(side_fill).inner_margin(Margin::symmetric(10, 3)).stroke(Stroke::new(1.0, line)))
                .show(ui, |ui| self.tab_bar(ui));
        }

        let zoom = self.tab_ref().map(|t| t.zoom);
        let zoom_to: Option<f32> = None;
        if let Some(z) = zoom_to {
            self.set_zoom(z);
        }

        if self.show_sidebar && !self.tabs.is_empty() {
            egui::Panel::left("thumbs")
                .resizable(true)
                .default_size(156.0)
                .min_size(124.0)
                .frame(egui::Frame::new().fill(side_fill).stroke(Stroke::new(1.0, line)))
                .show(ui, |ui| self.sidebar(ui));
        }
        if self.show_notes && !self.tabs.is_empty() {
            egui::Panel::right("notes")
                .resizable(true)
                .default_size(360.0)
                .min_size(260.0)
                .frame(egui::Frame::new().fill(side_fill).inner_margin(Margin::symmetric(14, 4)).stroke(Stroke::new(1.0, line)))
                .show(ui, |ui| self.notes_panel(ui));
        }
        let page_bg = pal.canvas;
        let mut canvas_rect = Rect::NOTHING;
        egui::CentralPanel::default().frame(egui::Frame::new().fill(page_bg)).show(ui, |ui| {
            canvas_rect = ui.max_rect();
            if self.tabs.is_empty() {
                self.welcome(ui);
                return;
            }
            let base = ViewCtx { tool: self.tool, color: self.color, dark_pages: self.dark_pages(), focused: false, touch: false };
            let user = self.settings.format.user_name.clone();
            let screenshot = self.screenshot_mode;
            let mut status = None;
            match self.right {
                None => {
                    let i = self.left;
                    let vc = ViewCtx { focused: true, ..base };
                    let (_, s) = page_view(ui, &mut self.tabs[i], vc, &user, screenshot, "pane0");
                    status = s;
                    #[cfg(test)]
                    {
                        self.debug_page_rect = self.tabs[i].debug_first_page;
                    }
                }
                Some(r) => {
                    let full = ui.available_rect_before_wrap();
                    let mid = full.center().x;
                    let left_rect = Rect::from_min_max(full.min, Pos2::new(mid - 1.0, full.max.y));
                    let right_rect = Rect::from_min_max(Pos2::new(mid + 1.0, full.min.y), full.max);
                    ui.painter().line_segment([Pos2::new(mid, full.top()), Pos2::new(mid, full.bottom())], Stroke::new(2.0, line));
                    for (idx, rect, is_right) in [(self.left, left_rect, false), (r, right_rect, true)] {
                        let focused = is_right == self.focus_right;
                        let vc = ViewCtx { focused, ..base };
                        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::top_down(Align::Min)));
                        child.set_clip_rect(rect);
                        let (touched, s) = page_view(&mut child, &mut self.tabs[idx], vc, &user, screenshot, if is_right { "pane2" } else { "pane1" });
                        if touched {
                            self.focus_right = is_right;
                        }
                        if s.is_some() {
                            status = s;
                        }
                        if focused {
                            ui.painter().rect_stroke(rect.shrink(1.0), 0.0, Stroke::new(2.0, ACCENT.gamma_multiply(0.5)), egui::StrokeKind::Inside);
                        }
                    }
                }
            }
            if let Some(s) = status {
                self.status = s;
            }
        });

        if let Some(z) = zoom {
            if let Some(nz) = zoom_pill(&ctx, z, canvas_rect, self.show_notes) {
                self.set_zoom(nz);
            }
        }
        self.toast(&ctx, canvas_rect);
        self.pending_popover(&ctx);
        self.edit_popover(&ctx);
        self.meta_window(&ctx);
        self.settings_window(&ctx);

        self.background_work(&ctx);
    }

    /// Debounced Obsidian sync, picking up memo edits made in Obsidian, and
    /// saving on quit.
    fn background_work(&mut self, ctx: &egui::Context) {
        let screenshot = self.screenshot_mode;
        if !screenshot {
            for t in &mut self.tabs {
                if t.note_checked.elapsed() < Duration::from_secs(2) || t.notes.synced_note.is_empty() {
                    continue;
                }
                t.note_checked = Instant::now();
                let path = PathBuf::from(&t.notes.synced_note);
                let Ok(mtime) = std::fs::metadata(&path).and_then(|m| m.modified()) else { continue };
                if Some(mtime) == t.note_mtime {
                    continue;
                }
                t.note_mtime = Some(mtime);
                let Ok(text) = std::fs::read_to_string(&path) else { continue };
                let file = obsidian::read_memo(&text);
                let merged = obsidian::merge_memo(&t.notes.memo, file.as_deref(), &t.notes.memo_synced);
                if merged != t.notes.memo {
                    t.notes.memo = merged.clone();
                    t.rev += 1;
                    t.sync_status = Some((true, "Obsidian에서 고친 메모를 가져왔어요.".into()));
                }
                if file.as_deref().map(str::trim) != Some(merged.trim()) {
                    // The app had changes too: write the combined memo back.
                    t.changed_at = Some(Instant::now());
                }
                t.notes.memo_synced = merged;
                t.notes.save(&t.path);
            }
            ctx.request_repaint_after(Duration::from_secs(2));
        }
        let mut wait = false;
        for t in &mut self.tabs {
            if let Some(at) = t.changed_at {
                if at.elapsed() > Duration::from_millis(1500) {
                    flush(t, &self.settings, screenshot, false);
                } else {
                    wait = true;
                }
            }
        }
        if wait {
            ctx.request_repaint_after(Duration::from_millis(300));
        }
        if ctx.input(|i| i.viewport().close_requested()) {
            for t in &mut self.tabs {
                flush(t, &self.settings, screenshot, true);
            }
        }
    }
}

// --------------------------------------------------------------- per-tab work

/// Something changed: save our record now, Obsidian shortly after.
fn changed(t: &mut Tab, screenshot: bool) {
    // Record an undo step when the annotations differ from the last step.
    // Typing a comment quickly counts as one step.
    if t.notes.annotations != t.snapshot {
        let typing_on = t.snapshot_at.elapsed() < Duration::from_millis(900)
            && t.snapshot.len() == t.notes.annotations.len()
            && t.snapshot.iter().zip(&t.notes.annotations).all(|(a, b)| a.id == b.id && a.kind == b.kind && a.color == b.color && a.rects == b.rects);
        if !typing_on {
            t.undo.push(std::mem::take(&mut t.snapshot));
            if t.undo.len() > 200 {
                t.undo.remove(0);
            }
        }
        t.snapshot = t.notes.annotations.clone();
        t.snapshot_at = Instant::now();
        t.redo.clear();
    }
    t.pdf_dirty = true;
    t.rev += 1;
    if !screenshot {
        t.notes.save(&t.path);
    }
    t.changed_at = Some(Instant::now());
}

/// ⌘Z / ⌘⇧Z: step the annotations back or forward.
fn undo_redo(t: &mut Tab, redo: bool, screenshot: bool) -> Option<String> {
    let state = if redo { t.redo.pop()? } else { t.undo.pop()? };
    let current = std::mem::replace(&mut t.notes.annotations, state);
    if redo { t.undo.push(current) } else { t.redo.push(current) }
    t.snapshot = t.notes.annotations.clone();
    t.snapshot_at = Instant::now() - Duration::from_secs(5);
    t.editing = None;
    t.pending = None;
    t.pdf_dirty = true;
    t.rev += 1;
    if !screenshot {
        t.notes.save(&t.path);
    }
    t.changed_at = Some(Instant::now());
    Some(crate::shortcuts::plat(if redo { "다시 했어요 (⌘⇧Z)" } else { "되돌렸어요 (⌘Z)" }))
}

/// Write pending work: the Obsidian note and (when closing) the PDF.
fn flush(t: &mut Tab, s: &Settings, screenshot: bool, closing: bool) {
    if screenshot {
        t.changed_at = None;
        return;
    }
    if t.changed_at.take().is_some() && s.obsidian.auto_sync && !s.obsidian.vault.is_empty() {
        sync_obsidian(t, s, screenshot, false);
    }
    if closing {
        save_pdf(t, s, screenshot);
    }
}

fn save_pdf(t: &mut Tab, s: &Settings, screenshot: bool) -> Option<String> {
    if screenshot || !s.save_into_pdf || !t.pdf_dirty {
        return None;
    }
    if t.pdf.save_with_annotations(&t.path, &t.notes.annotations, &PALETTE) {
        t.pdf_dirty = false;
        Some("PDF에 주석을 저장했어요.".into())
    } else {
        Some("PDF에 저장하지 못했어요 (읽기 전용일 수 있어요). 노트는 앱에 보관돼요.".into())
    }
}

fn sync_obsidian(t: &mut Tab, s: &Settings, screenshot: bool, open_after: bool) {
    if screenshot {
        return;
    }
    let result = {
        let pdf = &t.pdf;
        let label = |i: usize| pdf.page_label(i);
        obsidian::sync(&t.notes, &s.format, &s.obsidian, &label)
    };
    match result {
        Ok(r) => {
            let shown = r.note.strip_prefix(&s.obsidian.vault).map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|_| r.note.to_string_lossy().to_string());
            t.sync_status = Some((true, format!("{} 에 {}개 동기화 · {}", shown, r.count, chrono::Local::now().format("%H:%M"))));
            let p = r.note.to_string_lossy().to_string();
            let memo_changed = t.notes.memo != r.memo || t.notes.memo_synced != r.memo;
            if memo_changed {
                t.notes.memo = r.memo.clone();
                t.notes.memo_synced = r.memo;
                t.rev += 1;
            }
            t.note_mtime = std::fs::metadata(&r.note).and_then(|m| m.modified()).ok();
            if t.notes.synced_note != p || memo_changed {
                t.notes.synced_note = p;
                t.notes.save(&t.path);
            }
            if open_after {
                if let Err(e) = obsidian::open_in_obsidian(&r.note) {
                    t.sync_status = Some((false, format!("Obsidian을 열지 못했어요: {e}")));
                }
            }
        }
        Err(e) => t.sync_status = Some((false, e)),
    }
}

fn add_annotation(t: &mut Tab, kind: Kind, page: usize, rects: Vec<ViewRect>, text: String, color: usize, user: &str, screenshot: bool) -> u64 {
    let mut a = Annotation::new(kind, page, color, rects, text);
    a.author = user.to_string();
    let id = a.id;
    t.notes.annotations.push(a);
    changed(t, screenshot);
    id
}

/// Turn the waiting selection into an annotation of `kind`.
fn apply_pending(t: &mut Tab, kind: Kind, color: usize, user: &str, screenshot: bool) {
    let Some(pend) = t.pending.take() else { return };
    let real = if kind == Kind::Note { Kind::Highlight } else { kind };
    let id = add_annotation(t, real, pend.page, pend.rects, pend.text, color, user, screenshot);
    if kind == Kind::Note {
        t.editing = Some((id, pend.anchor));
    }
}

fn add_image_selection(t: &mut Tab, page: usize, r: ViewRect, color: usize, user: &str, screenshot: bool) -> Option<String> {
    let scale = 2.0;
    let img = t.pdf.render(page, scale)?;
    let x0 = ((r.x as f64) * scale).max(0.0) as usize;
    let y0 = ((r.y as f64) * scale).max(0.0) as usize;
    let x1 = (((r.x + r.w) as f64) * scale).min(img.width as f64) as usize;
    let y1 = (((r.y + r.h) as f64) * scale).min(img.height as f64) as usize;
    if x1 <= x0 + 2 || y1 <= y0 + 2 {
        return None;
    }
    let mut crop = image::RgbaImage::new((x1 - x0) as u32, (y1 - y0) as u32);
    for y in y0..y1 {
        let row = &img.rgba[(y * img.width + x0) * 4..(y * img.width + x1) * 4];
        for (k, px) in row.chunks_exact(4).enumerate() {
            crop.put_pixel(k as u32, (y - y0) as u32, image::Rgba([px[0], px[1], px[2], 255]));
        }
    }
    let alt = t.pdf.text_in_rect(page, &r);
    let stem = obsidian::safe_name(&obsidian::note_title(&t.notes)).replace(' ', "-");
    let file = format!("{}-p{}-{}.png", stem, page + 1, chrono::Local::now().format("%Y%m%d%H%M%S%3f"));
    let _ = std::fs::create_dir_all(images_dir());
    if crop.save(images_dir().join(&file)).is_err() {
        return Some("그림을 저장하지 못했어요.".into());
    }
    let id = add_annotation(t, Kind::Image, page, vec![r], alt, color, user, screenshot);
    if let Some(a) = t.notes.annotations.iter_mut().find(|a| a.id == id) {
        a.image_file = Some(file);
    }
    if !screenshot {
        t.notes.save(&t.path);
    }
    Some("이미지 선택을 추가했어요.".into())
}

fn store_done(t: &mut Tab, done: render::Done) {
    t.renderer.forget(&done.key);
    let Some(tex) = done.texture else { return };
    if done.key.level == render::THUMB {
        t.thumbs.insert(done.key, tex);
    } else {
        // A sharp picture for this page replaces its other zoom levels.
        t.pages.retain(|k, _| !(k.page == done.key.page && k.dark == done.key.dark && k.level != done.key.level));
        t.pages.insert(done.key, tex);
    }
}

/// Change the zoom keeping the document point under `anchor` (a screen
/// position inside the view) where it is.
fn zoom_about(t: &mut Tab, new_zoom: f32, anchor: Pos2) {
    let old = t.zoom;
    if t.viewport.is_positive() {
        let rel = anchor - t.viewport.min;
        let content = t.offset + rel;
        let f = new_zoom / old;
        let new_off = Vec2::new(content.x * f - rel.x, (content.y - PAGE_GAP) * f + PAGE_GAP - rel.y);
        t.set_offset = Some(Vec2::new(new_off.x.max(0.0), new_off.y.max(0.0)));
    }
    t.zoom = new_zoom;
    t.zoom_changed = Instant::now();
}

/// Draw one document's pages and handle the mouse on them. Returns whether
/// the person interacted with this view, and a status message.
fn page_view(ui: &mut egui::Ui, t: &mut Tab, vc: ViewCtx, user: &str, screenshot: bool, salt: &str) -> (bool, Option<String>) {
    let ppp = ui.ctx().pixels_per_point();
    let dark = vc.dark_pages;
    let mut status = None;
    let mut touched = false;

    while let Ok(done) = t.renderer.done.try_recv() {
        store_done(t, done);
    }

    let area = ui.available_rect_before_wrap();
    if vc.touch && (area.width() - t.last_width).abs() > 30.0 {
        t.fit_width = true;
    }
    t.last_width = area.width();
    // Newly shown: make the widest page fit the view (never above 100%).
    if t.fit_width && area.width() > 50.0 {
        let widest = t.sizes.iter().map(|s| s.0).fold(1.0f32, f32::max);
        let fit = ((area.width() - 2.0 * PAGE_GAP - 14.0) / widest).clamp(MIN_ZOOM, 1.0);
        if (fit - t.zoom).abs() > 0.01 {
            t.zoom = fit;
            t.zoom_changed = Instant::now() - ZOOM_SETTLE;
        }
        t.fit_width = false;
    }
    // Pinch on a trackpad, or ⌘/Ctrl + scroll wheel, zooms the document.
    let hover = ui.ctx().pointer_hover_pos().filter(|p| area.contains(*p));
    if let Some(p) = hover {
        let zd = ui.input(|i| i.zoom_delta());
        if (zd - 1.0).abs() > 1e-4 {
            let z = (t.zoom * zd).clamp(MIN_ZOOM, MAX_ZOOM);
            zoom_about(t, z, p);
            touched = true;
        }
    }
    let zoom = t.zoom;
    let settled = t.zoom_changed.elapsed() >= ZOOM_SETTLE;
    if !settled {
        ui.ctx().request_repaint_after(ZOOM_SETTLE);
    }
    let level = render::level_for(zoom * ppp);
    t.renderer.set_wanted_level(level);
    let hl_alpha = if dark { 95 } else { 115 };

    let scroll_target = t.scroll_to_page.take();
    let count = t.pdf.page_count();
    let scroll_target = scroll_target.map(|p| p.min(count.saturating_sub(1)));
    let mut new_current = t.current_page;
    let mut clicked: Option<(usize, (f32, f32), Pos2)> = None;
    let mut double: Option<(usize, (f32, f32), Pos2)> = None;
    let mut drag_started: Option<(usize, (f32, f32))> = None;
    let mut drag_now: Option<(usize, (f32, f32))> = None;
    let mut drag_released = false;
    let mut visible: Vec<usize> = Vec::new();
    let mut first_page_rect = None;

    let mut sa = egui::ScrollArea::both().id_salt(salt).auto_shrink([false, false]);
    if let Some(off) = t.set_offset.take() {
        sa = sa.scroll_offset(off);
    }
    let out = sa.show(ui, |ui| {
        let width = ui.available_width();
        ui.add_space(PAGE_GAP);
        let mut best_vis = f32::MIN;
        let view = ui.clip_rect();
        for p in 0..count {
            let (w, h) = t.sizes[p];
            let size = Vec2::new(w * zoom, h * zoom);
            let left_pad = ((width - size.x) / 2.0).max(PAGE_GAP);
            let (outer, _) = ui.allocate_exact_size(Vec2::new(left_pad + size.x + PAGE_GAP, size.y), Sense::hover());
            let rect = Rect::from_min_size(Pos2::new(outer.left() + left_pad, outer.top()), size);
            if p == 0 {
                first_page_rect = Some(rect);
            }
            if Some(p) == scroll_target {
                ui.scroll_to_rect(rect.expand(PAGE_GAP * 0.5), Some(Align::TOP));
            }
            let vis = rect.intersect(view).height();
            if vis > best_vis {
                best_vis = vis;
                new_current = p;
            }
            if !ui.is_rect_visible(rect.expand(300.0)) {
                ui.add_space(PAGE_GAP);
                continue;
            }
            visible.push(p);
            let key = render::Key { page: p, level, dark };
            if settled && !t.pages.contains_key(&key) {
                t.renderer.request(key, render::scale_of(level));
            }
            let painter = ui.painter_at(rect.expand(12.0));
            painter.rect_filled(rect.translate(Vec2::new(0.0, 3.0)).expand(2.0), 4.0, Color32::from_black_alpha(if dark { 90 } else { 30 }));
            // The sharp picture, or any other picture of this page meanwhile.
            let tex = t.pages.get(&key).or_else(|| {
                t.pages.iter().filter(|(k, _)| k.page == p && k.dark == dark).min_by_key(|(k, _)| (k.level as i64 - level as i64).abs()).map(|(_, v)| v)
            });
            let tex = tex.or_else(|| t.thumbs.get(&render::Key { page: p, level: render::THUMB, dark }));
            if let Some(tex) = tex {
                painter.image(tex.id(), rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
            } else {
                painter.rect_filled(rect, 0.0, if dark { Color32::from_gray(26) } else { Color32::WHITE });
            }

            let to_screen = |r: &ViewRect| Rect::from_min_size(rect.min + Vec2::new(r.x, r.y) * zoom, Vec2::new(r.w, r.h) * zoom);

            for a in t.notes.annotations.iter().filter(|a| a.page == p) {
                let c = color32(a.color);
                let selected = t.editing.map(|(id, _)| id) == Some(a.id);
                for r in &a.rects {
                    let sr = to_screen(r);
                    match a.kind {
                        Kind::Highlight => {
                            painter.rect_filled(sr, 2.0, Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), hl_alpha));
                        }
                        Kind::Underline => {
                            painter.line_segment([sr.left_bottom(), sr.right_bottom()], Stroke::new(1.8 * zoom.max(0.8), c));
                        }
                        Kind::Strike => {
                            let y = sr.center().y;
                            painter.line_segment([Pos2::new(sr.left(), y), Pos2::new(sr.right(), y)], Stroke::new(1.8 * zoom.max(0.8), c));
                        }
                        Kind::Image => {
                            painter.rect_filled(sr, 3.0, Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), 28));
                            painter.rect_stroke(sr, 3.0, Stroke::new(1.5, c), egui::StrokeKind::Inside);
                        }
                        Kind::Note => {
                            let center = sr.left_top() + Vec2::splat(9.0 * zoom.max(0.7));
                            let rr = Rect::from_center_size(center, Vec2::splat(20.0 * zoom.max(0.7)));
                            painter.rect_filled(rr.translate(Vec2::new(0.0, 1.5)), 5.0, Color32::from_black_alpha(40));
                            painter.rect_filled(rr, 5.0, c);
                            icons::paint(&painter, rr.shrink(3.0), Icon::Note, Color32::from_black_alpha(170), c);
                        }
                    }
                    if selected {
                        painter.rect_stroke(sr.expand(2.0), 3.0, Stroke::new(1.5, ACCENT), egui::StrokeKind::Outside);
                    }
                }
                if !a.comment.is_empty() && a.kind != Kind::Note {
                    if let Some(r) = a.rects.first() {
                        let sr = to_screen(r);
                        painter.circle_filled(Pos2::new(rect.left() + 9.0, sr.center().y), 4.0, c);
                    }
                }
            }

            if t.search_hits.contains(&p) {
                painter.rect_stroke(rect.expand(4.0), 4.0, Stroke::new(2.0, LINK), egui::StrokeKind::Outside);
            }

            // Live drag preview.
            let preview: Option<(Vec<ViewRect>, Color32)> = match &t.drag {
                Some(Drag::Text { page, start, now }) if *page == p => {
                    let (s, n) = (*start, *now);
                    let c = match vc.tool {
                        Tool::Select | Tool::Note | Tool::Image => Color32::from_rgba_unmultiplied(63, 140, 255, 70),
                        _ => {
                            let c = color32(vc.color);
                            Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), hl_alpha)
                        }
                    };
                    t.select(p, s, n).map(|r| (r, c))
                }
                _ => None,
            };
            if let Some((rects, c)) = preview {
                for r in rects {
                    painter.rect_filled(to_screen(&r), 1.0, c);
                }
            }
            if let Some(Drag::Area { page, start, now }) = &t.drag {
                if *page == p {
                    let r = ViewRect::from_points(*start, *now);
                    painter.rect_filled(to_screen(&r), 2.0, ACCENT.gamma_multiply(0.08));
                    painter.rect_stroke(to_screen(&r), 2.0, Stroke::new(1.5, ACCENT), egui::StrokeKind::Inside);
                }
            }
            if let Some(pend) = &t.pending {
                if pend.page == p {
                    for r in &pend.rects {
                        painter.rect_filled(to_screen(r), 1.0, Color32::from_rgba_unmultiplied(63, 140, 255, 70));
                    }
                }
            }

            let marking = !matches!(vc.tool, Tool::Select | Tool::Note);
            let sense = if vc.touch && !marking { Sense::click() } else { Sense::click_and_drag() };
            let resp = ui.interact(rect, Id::new((salt, "page", p)), sense);
            if vc.touch && resp.long_touched() {
                if let Some(pos) = resp.interact_pointer_pos() {
                    double = Some((p, ((pos.x - rect.left()) / zoom, (pos.y - rect.top()) / zoom), pos));
                }
            }
            let cursor = if vc.tool == Tool::Image { egui::CursorIcon::Crosshair } else { egui::CursorIcon::Text };
            let resp = resp.on_hover_cursor(cursor);
            let to_view = |pos: Pos2| ((pos.x - rect.left()) / zoom, (pos.y - rect.top()) / zoom);
            if resp.drag_started() {
                if let Some(pos) = resp.interact_pointer_pos() {
                    drag_started = Some((p, to_view(pos)));
                }
            }
            if resp.dragged() {
                if let Some(pos) = ui.ctx().pointer_latest_pos() {
                    let v = to_view(pos);
                    drag_now = Some((p, (v.0.clamp(0.0, w), v.1.clamp(0.0, h))));
                }
            }
            if resp.drag_stopped() {
                drag_released = true;
            }
            if resp.double_clicked() {
                if let Some(pos) = resp.interact_pointer_pos() {
                    double = Some((p, to_view(pos), pos));
                }
            } else if resp.clicked() {
                if let Some(pos) = resp.interact_pointer_pos() {
                    clicked = Some((p, to_view(pos), pos));
                }
            }
            ui.add_space(PAGE_GAP);
        }
    });
    t.offset = out.state.offset;
    t.viewport = out.inner_rect;
    t.current_page = new_current;
    #[cfg(test)]
    {
        t.debug_first_page = first_page_rect;
    }
    let _ = first_page_rect;

    // Keep memory small: drop pictures of pages far from the view.
    if let (Some(&lo), Some(&hi)) = (visible.first(), visible.last()) {
        let (lo, hi) = (lo.saturating_sub(3), hi + 3);
        t.pages.retain(|k, _| k.page >= lo && k.page <= hi && k.dark == dark);
    }

    let tool = vc.tool;
    let color = vc.color;
    if drag_started.is_some() || clicked.is_some() || double.is_some() {
        touched = true;
    }
    if !vc.focused && touched {
        // First click on the other document only moves the focus there…
        if clicked.is_some() {
            return (true, None);
        }
    }
    if let Some((p, start)) = drag_started {
        t.pending = None;
        t.editing = None;
        t.drag = Some(if tool == Tool::Image { Drag::Area { page: p, start, now: start } } else { Drag::Text { page: p, start, now: start } });
    }
    if let Some((p, v)) = drag_now {
        match &mut t.drag {
            Some(Drag::Text { page, now, .. }) | Some(Drag::Area { page, now, .. }) if *page == p => *now = v,
            _ => {}
        }
    }
    if drag_released {
        let pointer = ui.ctx().pointer_latest_pos().unwrap_or_default();
        match t.drag.take() {
            Some(Drag::Text { page, start, now }) => {
                if let Some((text, rects)) = t.pdf.select_text(page, start, now) {
                    match tool {
                        Tool::Highlight | Tool::Underline | Tool::Strike => {
                            add_annotation(t, tool.kind().unwrap(), page, rects, text, color, user, screenshot);
                            status = Some(format!("{}을(를) 추가했어요.", tool.kind().unwrap().label()));
                        }
                        _ => t.pending = Some(Pending { page, text, rects, anchor: pointer }),
                    }
                } else {
                    status = Some("이 부분에서는 글자를 찾지 못했어요 (스캔한 PDF는 이미지 선택을 써 보세요).".into());
                }
                t.sel_cache = None;
            }
            Some(Drag::Area { page, start, now }) => {
                let r = ViewRect::from_points(start, now);
                if r.w > 4.0 && r.h > 4.0 {
                    status = add_image_selection(t, page, r, color, user, screenshot);
                }
            }
            None => {}
        }
    }
    if let Some((p, v, screen)) = double {
        if let Some((text, rects)) = t.pdf.word_at(p, v) {
            if !matches!(tool, Tool::Highlight | Tool::Underline | Tool::Strike) {
                t.pending = Some(Pending { page: p, text, rects, anchor: screen });
            }
        }
    }
    if let Some((p, v, screen)) = clicked {
        let hit = t.notes.annotations.iter().rev().find(|a| a.page == p && a.hit(v)).map(|a| a.id);
        t.pending = None;
        if let Some(id) = hit {
            t.editing = Some((id, screen));
        } else {
            t.editing = None;
            match tool {
                Tool::Note => {
                    let id = add_annotation(t, Kind::Note, p, vec![ViewRect { x: v.0 - 9.0, y: v.1 - 9.0, w: 18.0, h: 18.0 }], String::new(), color, user, screenshot);
                    t.editing = Some((id, screen));
                }
                Tool::Highlight | Tool::Underline | Tool::Strike => {
                    if let Some((text, rects)) = t.pdf.word_at(p, v) {
                        add_annotation(t, tool.kind().unwrap(), p, rects, text, color, user, screenshot);
                        status = Some(format!("{}을(를) 추가했어요.", tool.kind().unwrap().label()));
                    }
                }
                _ => {}
            }
        }
    }
    (touched, status)
}

/// Author, institution, year and note name from the PDF (keeps what's set).
fn fill_details(pdf: &PdfDoc, notes: &mut DocNotes, template: &str) {
    let first = pdf.page_text(0);
    if notes.title.trim().is_empty() || looks_like_junk_title(&notes.title) {
        if let Some(t) = guess_title(&first) {
            notes.title = t;
        }
    }
    if notes.author.trim().is_empty() {
        if let Some(a) = naming::authors_from_text(&first) {
            notes.author = naming::split_authors(&a).join(", ");
        }
    }
    if notes.institution.trim().is_empty() {
        notes.institution = naming::institution_from_text(&first).unwrap_or_default();
    }
    if notes.year.trim().is_empty() {
        let this_year = chrono::Datelike::year(&chrono::Local::now());
        notes.year = naming::year_from(&first, pdf.creation_year(), this_year).map(|y| y.to_string()).unwrap_or_default();
    }
    if notes.doi.trim().is_empty() {
        if let Some(doi) = find_doi(&first) {
            notes.doi = doi;
        }
    }
    notes.note_name = auto_name(notes, template);
}

/// PDF title fields often hold "Microsoft Word - draft3.docx" and the like.
fn looks_like_junk_title(t: &str) -> bool {
    let l = t.to_lowercase();
    l.starts_with("microsoft word") || l.ends_with(".doc") || l.ends_with(".docx") || l.ends_with(".pdf") || l.starts_with("untitled") || l.trim().len() < 4
}

/// The paper's title is usually the first real line of page 1.
fn guess_title(first_page: &str) -> Option<String> {
    let skip = ["doi", "http", "journal", "vol.", "volume", "©", "copyright", "issn", "received", "article", "research paper", "original"];
    first_page.lines().map(str::trim).take(8).find(|l| {
        let low = l.to_lowercase();
        let n = l.chars().count();
        (8..=220).contains(&n) && !skip.iter().any(|k| low.contains(k)) && l.chars().filter(|c| c.is_alphabetic()).count() * 2 > n
    }).map(|s| s.to_string())
}

/// The note name made from the pattern, or the title / file name.
fn auto_name(notes: &DocNotes, template: &str) -> String {
    let file = Path::new(&notes.pdf_path).file_stem().map(|s| crate::pdf::nfc(&s.to_string_lossy())).unwrap_or_default();
    let n = naming::render(template, &notes.author, &notes.institution, &notes.year, &notes.title, &file);
    if !n.is_empty() {
        n
    } else if !notes.title.trim().is_empty() {
        notes.title.trim().to_string()
    } else {
        file
    }
}

fn tool_action(t: Tool) -> Action {
    match t {
        Tool::Select => Action::Select,
        Tool::Strike => Action::Strike,
        Tool::Underline => Action::Underline,
        Tool::Highlight => Action::Highlight,
        Tool::Note => Action::Note,
        Tool::Image => Action::Capture,
    }
}

fn start_cite_fetch(doi: &str, ctx: &egui::Context) -> Receiver<Result<citation::Found, String>> {
    let (tx, rx) = std::sync::mpsc::channel();
    let doi = doi.to_string();
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let _ = tx.send(citation::fetch_crossref(&doi));
        ctx.request_repaint();
    });
    rx
}

/// Use what CrossRef knows (it is more reliable than PDF details).
fn apply_found(n: &mut DocNotes, f: citation::Found, template: &str) {
    if !f.title.trim().is_empty() {
        n.title = crate::pdf::nfc(f.title.trim());
    }
    if !f.authors.is_empty() {
        n.author = f.authors.iter().map(|p| if p.given.is_empty() { p.family.clone() } else { format!("{} {}", p.given, p.family) }).collect::<Vec<_>>().join(", ");
        n.authors = f.authors;
    }
    for (dst, src) in [(&mut n.year, f.year), (&mut n.journal, f.journal), (&mut n.volume, f.volume), (&mut n.issue, f.issue), (&mut n.pages, f.pages), (&mut n.publisher, f.publisher)] {
        if !src.trim().is_empty() {
            *dst = src.trim().to_string();
        }
    }
    if !n.note_name_custom {
        n.note_name = auto_name(n, template);
    }
}

/// The person's own memo about the document, kept in step with the
/// "내 메모" section of the Obsidian note.
fn memo_card(ui: &mut egui::Ui, t: &mut Tab, s: &Settings, changed_any: &mut bool) {
    let pal = theme::pal(ui.ctx());
    let r = ui.add(
        egui::TextEdit::multiline(&mut t.notes.memo)
            .hint_text("메모 추가…")
            .desired_rows(3)
            .desired_width(f32::INFINITY)
            .margin(Margin::same(10))
            .background_color(ui.visuals().faint_bg_color)
            .frame(egui::Frame::new().corner_radius(CornerRadius::same(8)))
            .id_salt(("memo", t.path.clone())),
    );
    if r.changed() {
        *changed_any = true;
    }
    if r.has_focus() {
        ui.painter().rect_stroke(r.rect, 8.0, Stroke::new(1.0, pal.accent.gamma_multiply(0.6)), egui::StrokeKind::Inside);
    }
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        let page = t.current_page;
        let label = t.pdf.page_label(page);
        if link_label(ui, &format!("+ p. {label} 링크")).on_hover_text("지금 보는 쪽으로 가는 링크").clicked() {
            let link = if s.obsidian.copy_pdf && s.obsidian.link_style == LinkStyle::ObsidianPdf {
                let name = t.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                let folder = s.obsidian.attachments_folder.trim_matches('/');
                let target = if folder.is_empty() { name } else { format!("{folder}/{name}") };
                format!("[[{}#page={}|p.{}]]", target, page + 1, label)
            } else {
                format!("(p.{label})")
            };
            append_line(&mut t.notes.memo, &link);
            *changed_any = true;
        }
        if link_label(ui, "+ 날짜").clicked() {
            append_line(&mut t.notes.memo, &chrono::Local::now().format("%Y-%m-%d").to_string());
            *changed_any = true;
        }
        if let Some(a) = t.editing.and_then(|(id, _)| t.notes.annotations.iter().find(|a| a.id == id)) {
            if !a.text.is_empty() && link_label(ui, "+ 고른 문장").clicked() {
                let q = format!("> {} (p.{})", a.text.trim(), t.pdf.page_label(a.page));
                append_line(&mut t.notes.memo, &q);
                *changed_any = true;
            }
        }
    });
}

fn append_line(text: &mut String, line: &str) {
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(line);
    text.push('\n');
}

/// "How to cite", folded away until wanted.
fn cite_card(ui: &mut egui::Ui, t: &Tab, s: &Settings, fetch: &mut bool) {
    let pal = theme::pal(ui.ctx());
    let w = citation::Work::from_notes(&t.notes);
    egui::CollapsingHeader::new(RichText::new("인용").family(icons::bold()).size(12.0).color(pal.weak)).id_salt(("cite", t.path.clone())).default_open(false).show(ui, |ui| {
        let row = |ui: &mut egui::Ui, name: &str, text: String, copy: String| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(name).size(11.0).color(pal.weak));
                ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                    if icons::small(ui, Icon::Copy, false, ACCENT, "복사").clicked() {
                        ui.ctx().copy_text(copy);
                    }
                });
            });
            ui.label(RichText::new(text).size(12.5));
        };
        row(ui, "본문", format!("{}  ·  {}", w.in_text(), w.narrative()), w.in_text());
        row(ui, "APA 7", w.apa().replace('*', ""), w.apa().replace('*', ""));
        if s.format.citation.mla {
            row(ui, "MLA 9", w.mla().replace('*', ""), w.mla().replace('*', ""));
        }
        if s.format.citation.chicago {
            row(ui, "Chicago", w.chicago().replace('*', ""), w.chicago().replace('*', ""));
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            if link_label(ui, "BibTeX 복사").clicked() {
                ui.ctx().copy_text(w.bibtex());
            }
            if link_label(ui, "DOI로 채우기").on_hover_text("CrossRef에서 학술지·권·호·쪽을 가져와요 (DOI만 보냅니다)").clicked() {
                *fetch = true;
            }
        });
        if let Some(st) = &t.cite_status {
            ui.label(RichText::new(st).size(11.0).color(pal.weak));
        }
    });
}

/// What the AI reads: details, my highlights and memo, then the text with
/// page marks (cut to `limit` characters).
fn ai_context(t: &Tab, limit: usize) -> String {
    let mut s = String::new();
    let n = &t.notes;
    s.push_str(&format!("Title: {}\n", if n.title.trim().is_empty() { t.name() } else { n.title.trim().to_string() }));
    for (k, v) in [("Authors", &n.author), ("Institution", &n.institution), ("Year", &n.year), ("Journal", &n.journal), ("DOI", &n.doi)] {
        if !v.trim().is_empty() {
            s.push_str(&format!("{k}: {}\n", v.trim()));
        }
    }
    if !n.annotations.is_empty() {
        s.push_str("\n## Reader's highlights\n");
        for a in n.sorted() {
            let label = t.pdf.page_label(a.page);
            let text = if a.text.is_empty() { "(note)".to_string() } else { a.text.trim().to_string() };
            s.push_str(&format!("- [p. {label}] {text}"));
            if !a.comment.trim().is_empty() {
                s.push_str(&format!(" — reader's comment: {}", a.comment.trim()));
            }
            s.push('\n');
        }
    }
    if !n.memo.trim().is_empty() {
        s.push_str(&format!("\n## Reader's memo\n{}\n", n.memo.trim()));
    }
    s.push_str("\n## Full text\n");
    for p in 0..t.pdf.page_count() {
        let page = t.pdf.page_text(p);
        let block = format!("\n[p. {}]\n{}\n", t.pdf.page_label(p), page.trim());
        if s.len() + block.len() > limit {
            s.push_str("\n[… the rest of the document was cut for length …]\n");
            break;
        }
        s.push_str(&block);
    }
    s
}

fn t_ai_name(p: Option<ai::Provider>) -> &'static str {
    p.map(|p| p.short()).unwrap_or("AI")
}

/// Light Markdown: headings, bullets, **bold** spans.
fn render_md(ui: &mut egui::Ui, text: &str, pal: &theme::Pal) {
    for line in text.lines() {
        let l = line.trim_end();
        if l.trim().is_empty() {
            ui.add_space(4.0);
            continue;
        }
        let (body, size, bold, indent) = if let Some(h) = l.strip_prefix("### ").or(l.strip_prefix("## ")).or(l.strip_prefix("# ")) {
            (h.to_string(), 13.5, true, 0.0)
        } else if let Some(b) = l.trim_start().strip_prefix("- ").or(l.trim_start().strip_prefix("* ")) {
            (format!("•  {b}"), 13.0, false, 4.0 + (l.len() - l.trim_start().len()) as f32 * 4.0)
        } else {
            (l.to_string(), 13.0, false, 0.0)
        };
        ui.horizontal_wrapped(|ui| {
            ui.add_space(indent);
            ui.spacing_mut().item_spacing.x = 0.0;
            for (i, part) in body.split("**").enumerate() {
                if part.is_empty() {
                    continue;
                }
                let mut rt = RichText::new(part).size(size).color(pal.text);
                if bold || i % 2 == 1 {
                    rt = rt.family(icons::bold());
                }
                ui.label(rt);
            }
        });
    }
}

/// Small text that acts like a link.
fn link_label(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let pal = theme::pal(ui.ctx());
    let id = ui.next_auto_id();
    let hovered = ui.ctx().read_response(id).map(|r| r.hovered()).unwrap_or(false);
    let r = ui.add(egui::Label::new(RichText::new(text).size(11.5).color(if hovered { pal.accent } else { pal.weak })).sense(Sense::click()));
    r.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn divider(ui: &mut egui::Ui, pal: &theme::Pal) {
    ui.add_space(14.0);
    let r = ui.available_rect_before_wrap();
    ui.painter().hline(r.left()..=r.right(), r.top(), Stroke::new(1.0, pal.border));
    ui.add_space(14.0);
}

fn section_title(ui: &mut egui::Ui, title: &str, count: Option<usize>, pal: &theme::Pal) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).family(icons::bold()).size(12.0).color(pal.weak));
        if let Some(n) = count {
            if n > 0 {
                ui.label(RichText::new(n.to_string()).size(11.0).color(pal.weak));
            }
        }
    });
    ui.add_space(4.0);
}

/// A small "−  100%  +" control floating at the bottom right of the pages.
fn zoom_pill(ctx: &egui::Context, z: f32, area: Rect, _notes: bool) -> Option<f32> {
    if !area.is_positive() {
        return None;
    }
    let pal = theme::pal(ctx);
    let mut out = None;
    egui::Area::new(Id::new("zoom-pill"))
        .order(Order::Foreground)
        .fixed_pos(Pos2::new(area.right() - 16.0, area.bottom() - 16.0))
        .pivot(Align2::RIGHT_BOTTOM)
        .show(ctx, |ui| {
            let near = ctx.pointer_hover_pos().map(|p| p.distance(Pos2::new(area.right() - 60.0, area.bottom() - 30.0)) < 140.0).unwrap_or(false);
            ui.set_opacity(if near { 1.0 } else { 0.55 });
            egui::Frame::new()
                .fill(pal.card)
                .stroke(Stroke::new(1.0, pal.border))
                .corner_radius(CornerRadius::same(9))
                .inner_margin(Margin::symmetric(4, 2))
                .shadow(egui::epaint::Shadow { offset: [0, 2], blur: 10, spread: 0, color: Color32::from_black_alpha(25) })
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        let b = |t: &str| egui::Button::new(RichText::new(t).size(14.0).color(pal.weak)).fill(Color32::TRANSPARENT).min_size(Vec2::new(26.0, 22.0));
                        if ui.add(b("−")).on_hover_text(crate::shortcuts::plat("축소  ⌘−")).clicked() {
                            out = Some(z / 1.15);
                        }
                        if ui.add(egui::Button::new(RichText::new(format!("{:.0}%", z * 100.0)).size(11.5).color(pal.text)).fill(Color32::TRANSPARENT).min_size(Vec2::new(46.0, 22.0))).on_hover_text(crate::shortcuts::plat("100%로  ⌘0")).clicked() {
                            out = Some(1.0);
                        }
                        if ui.add(b("+")).on_hover_text(crate::shortcuts::plat("확대  ⌘+")).clicked() {
                            out = Some(z * 1.15);
                        }
                    });
                });
        });
    out
}

/// PDFs in the app's own Documents folder (on iPhone, what the Files app puts
/// in "PDF Notes"), newest first.
fn phone_library() -> Vec<PathBuf> {
    let dir = documents_dir();
    let mut v: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(&dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.extension().map(|x| x.eq_ignore_ascii_case("pdf")).unwrap_or(false))
                .map(|p| (std::fs::metadata(&p).and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH), p))
                .collect()
        })
        .unwrap_or_default();
    v.sort_by(|a, b| b.0.cmp(&a.0));
    v.into_iter().map(|(_, p)| p).collect()
}

pub fn documents_dir() -> PathBuf {
    crate::model::home_dir().join("Documents")
}

fn segmented_wide(ui: &mut egui::Ui, value: &mut bool, options: &[(bool, &str)]) {
    let pal = theme::pal(ui.ctx());
    let w = ui.available_width();
    egui::Frame::new().fill(ui.visuals().faint_bg_color).corner_radius(CornerRadius::same(9)).inner_margin(Margin::same(2)).show(ui, |ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.horizontal(|ui| {
            let each = (w - 4.0) / options.len() as f32;
            for (v, label) in options {
                let sel = *value == *v;
                let b = egui::Button::new(RichText::new(*label).size(13.0).family(icons::bold()).color(if sel { pal.text } else { pal.weak }))
                    .fill(if sel { pal.card } else { Color32::TRANSPARENT })
                    .corner_radius(CornerRadius::same(7))
                    .min_size(Vec2::new(each, 26.0));
                if ui.add(b).clicked() {
                    *value = *v;
                }
            }
        });
    });
}

fn popover_frame(ui: &egui::Ui) -> egui::Frame {
    let dark = ui.visuals().dark_mode;
    egui::Frame::popup(ui.style())
        .fill(if dark { Color32::from_rgb(40, 40, 44) } else { Color32::WHITE })
        .corner_radius(CornerRadius::same(14))
        .inner_margin(Margin::same(10))
}

fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(8.0);
    ui.label(RichText::new(title).family(icons::bold()).size(13.5));
    ui.add_space(2.0);
}

fn segmented<T: PartialEq + Copy>(ui: &mut egui::Ui, value: &mut T, options: &[(T, &str)]) {
    egui::Frame::new().fill(ui.visuals().faint_bg_color).corner_radius(CornerRadius::same(8)).inner_margin(Margin::same(2)).show(ui, |ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for (v, label) in options.iter().rev() {
            let sel = *value == *v;
            let b = egui::Button::new(RichText::new(*label).size(12.0).color(if sel { ui.visuals().strong_text_color() } else { ui.visuals().weak_text_color() }))
                .fill(if sel { ui.visuals().extreme_bg_color } else { Color32::TRANSPARENT })
                .corner_radius(CornerRadius::same(6));
            if ui.add(b).clicked() {
                *value = *v;
            }
        }
    });
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n { s.to_string() } else { format!("{}…", s.chars().take(n).collect::<String>()) }
}

fn find_doi(text: &str) -> Option<String> {
    let lower = text.to_lowercase();
    let i = lower.find("10.")?;
    let rest = &text[i..];
    let doi: String = rest.chars().take_while(|c| !c.is_whitespace() && *c != ',' && *c != ';').collect();
    let doi = doi.trim_end_matches(['.', ')', ']']).to_string();
    if doi.contains('/') { Some(doi) } else { None }
}

fn paint_dot(ui: &mut egui::Ui, i: usize) {
    let (r, _) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
    ui.painter().circle_filled(r.center(), 5.5, color32(i));
}

fn color_dot(ui: &mut egui::Ui, i: usize, selected: bool) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(Vec2::splat(30.0), Sense::click());
    if selected {
        ui.painter().rect_filled(r.shrink(1.0), 8.0, ui.visuals().widgets.inactive.weak_bg_fill);
    }
    ui.painter().circle_filled(r.center(), 8.5, color32(i));
    if resp.hovered() {
        ui.painter().circle_stroke(r.center(), 10.5, Stroke::new(1.2, ui.visuals().weak_text_color()));
    }
    resp.on_hover_text(COLOR_NAMES_KO[i])
}

fn install_fonts(ctx: &egui::Context) {
    // Pretendard for Latin and Hangul; the computer's own CJK font for Hanja,
    // Japanese and Chinese, which Korean papers use often.
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert("pretendard".into(), std::sync::Arc::new(egui::FontData::from_static(include_bytes!("../fonts/Pretendard-Regular.otf"))));
    fonts.font_data.insert("pretendard-semibold".into(), std::sync::Arc::new(egui::FontData::from_static(include_bytes!("../fonts/Pretendard-SemiBold.otf"))));
    let mut cjk: Vec<String> = Vec::new();
    // One CJK face covers every Hanja (traditional and simplified) and kana;
    // more of them cost >100 MB of memory for nothing Pretendard lacks.
    // The first one this computer has is used.
    let candidates: Vec<std::path::PathBuf> = if cfg!(windows) {
        let dir = std::env::var_os("WINDIR").map(std::path::PathBuf::from).unwrap_or_else(|| "C:\\Windows".into()).join("Fonts");
        ["msyh.ttc", "msjh.ttc", "YuGothR.ttc", "simsun.ttc", "malgun.ttf"].iter().map(|f| dir.join(f)).collect()
    } else if cfg!(target_os = "macos") || cfg!(target_os = "ios") {
        vec!["/System/Library/Fonts/Hiragino Sans GB.ttc".into()]
    } else {
        ["/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc", "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc", "/usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc"].iter().map(Into::into).collect()
    };
    for path in candidates {
        if let Ok(bytes) = std::fs::read(&path) {
            fonts.font_data.insert("system-cjk".into(), std::sync::Arc::new(egui::FontData::from_owned(bytes)));
            cjk.push("system-cjk".into());
            break;
        }
    }
    let prop = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let mono = fonts.families.get(&FontFamily::Monospace).cloned().unwrap_or_default();
    let chain = |first: &[&str], rest: &[String]| -> Vec<String> {
        let mut v: Vec<String> = first.iter().map(|s| s.to_string()).collect();
        v.extend(cjk.iter().cloned());
        v.extend(rest.iter().cloned());
        v
    };
    fonts.families.insert(FontFamily::Proportional, chain(&["pretendard"], &prop));
    fonts.families.insert(icons::bold(), chain(&["pretendard-semibold", "pretendard"], &prop));
    let mut m = mono.clone();
    m.push("pretendard".into());
    m.extend(cjk.iter().cloned());
    m.extend(prop.iter().cloned());
    fonts.families.insert(FontFamily::Monospace, m);
    ctx.set_fonts(fonts);
}


#[cfg(test)]
mod ui_tests {
    use super::*;
    use egui_kittest::{kittest::Queryable, Harness};

    fn setup(names: &[&str]) -> Harness<'static, App> {
        let work = std::env::temp_dir().join("pnt-ui");
        let _ = std::fs::create_dir_all(&work);
        let mut paths = Vec::new();
        for n in names {
            let pdf = work.join(format!("{n}.pdf"));
            std::fs::copy(crate::SAMPLE_PDF, &pdf).unwrap();
            paths.push(pdf);
        }
        let scratch = egui::Context::default();
        let app = App::new(&scratch, paths, true);
        let mut ready = false;
        let mut h = Harness::builder().with_size(Vec2::new(1280.0, 820.0)).build_ui_state(
            move |ui, app: &mut App| {
                if !ready {
                    app.prepare(ui.ctx());
                    ready = true;
                    return;
                }
                app.ui(ui)
            },
            app,
        );
        h.run_steps(8);
        h
    }

    fn drag(h: &mut Harness<'static, App>, a: Pos2, b: Pos2) {
        h.hover_at(a);
        h.step();
        h.event(egui::Event::PointerButton { pos: a, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
        h.step();
        for i in 1..=10 {
            h.event(egui::Event::PointerMoved(a + (b - a) * (i as f32 / 10.0)));
            h.step();
        }
        h.event(egui::Event::PointerButton { pos: b, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
        h.run_steps(3);
    }

    fn count(h: &Harness<'static, App>) -> usize {
        h.state().tab_ref().unwrap().notes.annotations.len()
    }

    const HL: &str = "형광펜 — 글을 끌거나 단어를 클릭하세요";

    fn ok() -> bool {
        Path::new(crate::SAMPLE_PDF).exists()
    }

    #[test]
    fn drag_with_highlight_tool_adds_a_highlight() {
        if !ok() {
            return;
        }
        let mut h = setup(&["a"]);
        h.get_by_label(HL).click();
        h.run_steps(2);
        assert!(h.state().tool == Tool::Highlight);
        let page = h.state().debug_page_rect.expect("page drawn");
        drag(&mut h, page.min + Vec2::new(105.0, 145.0), page.min + Vec2::new(400.0, 170.0));
        assert_eq!(count(&h), 1);
    }

    #[test]
    fn select_then_press_highlight_button() {
        if !ok() {
            return;
        }
        let mut h = setup(&["b"]);
        let page = h.state().debug_page_rect.expect("page drawn");
        drag(&mut h, page.min + Vec2::new(105.0, 145.0), page.min + Vec2::new(400.0, 170.0));
        assert!(h.state().tab_ref().unwrap().pending.is_some(), "selection should wait for a choice");
        h.get_by_label(HL).click();
        h.run_steps(3);
        assert_eq!(count(&h), 1);
        assert_eq!(h.state().tab_ref().unwrap().notes.annotations[0].kind, Kind::Highlight);
    }

    #[test]
    fn click_a_word_with_highlight_tool() {
        if !ok() {
            return;
        }
        let mut h = setup(&["c"]);
        h.state_mut().tool = Tool::Highlight;
        h.run_steps(2);
        let page = h.state().debug_page_rect.expect("page drawn");
        let p = page.min + Vec2::new(150.0, 160.0);
        h.hover_at(p);
        h.step();
        h.event(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
        h.step();
        h.event(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
        h.run_steps(3);
        assert_eq!(count(&h), 1);
    }

    #[test]
    fn pinch_and_ctrl_scroll_zoom_the_document() {
        if !ok() {
            return;
        }
        let mut h = setup(&["d"]);
        let page = h.state().debug_page_rect.expect("page drawn");
        h.hover_at(page.center());
        h.step();
        h.event(egui::Event::Zoom(1.5));
        h.run_steps(3);
        let z = h.state().tab_ref().unwrap().zoom;
        assert!((z - 1.5).abs() < 0.01, "pinch zoom gave {z}");
        h.event(egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Line, delta: Vec2::new(0.0, -3.0), modifiers: egui::Modifiers::COMMAND, phase: egui::TouchPhase::Move });
        h.run_steps(3);
        let z2 = h.state().tab_ref().unwrap().zoom;
        assert!(z2 < z, "⌘+scroll down should zoom out ({z} -> {z2})");
    }

    fn key(h: &mut Harness<'static, App>, key: Key, physical: Key, modifiers: egui::Modifiers) {
        h.event(egui::Event::Key { key, physical_key: Some(physical), pressed: true, repeat: false, modifiers });
        h.step();
        h.event(egui::Event::Key { key, physical_key: Some(physical), pressed: false, repeat: false, modifiers });
        h.run_steps(2);
    }

    #[test]
    fn shortcuts_highlight_capture_and_switch_documents() {
        if !ok() {
            return;
        }
        let mut h = setup(&["k1", "k2"]);
        // Close the side-by-side view the screenshot mode opened.
        h.state_mut().right = None;
        h.state_mut().focus_right = false;
        h.run_steps(3);
        let page = h.state().debug_page_rect.expect("page drawn");
        let s = page.width() / 612.0;
        drag(&mut h, page.min + Vec2::new(105.0, 145.0) * s, page.min + Vec2::new(400.0, 170.0) * s);
        assert!(h.state().tab_ref().unwrap().pending.is_some());
        // H with the Korean input source on: the key reads "ㅗ" but sits where H is.
        key(&mut h, Key::Num0, Key::H, egui::Modifiers::NONE);
        assert_eq!(count(&h), 1, "H highlights the selection");
        assert_eq!(h.state().tab_ref().unwrap().notes.annotations[0].kind, Kind::Highlight);
        // C toggles the capture tool.
        key(&mut h, Key::C, Key::C, egui::Modifiers::NONE);
        assert!(h.state().tool == Tool::Image);
        key(&mut h, Key::C, Key::C, egui::Modifiers::NONE);
        assert!(h.state().tool == Tool::Select);
        // ⌘⇧] goes to the next document.
        let before = h.state().active();
        key(&mut h, Key::CloseBracket, Key::CloseBracket, egui::Modifiers::COMMAND | egui::Modifiers::SHIFT);
        assert_ne!(h.state().active(), before);
        // A changed shortcut works and the old one doesn't.
        let b = shortcuts::Binding { key: "F2".into(), command: false, shift: false, alt: false, ctrl: false };
        h.state_mut().settings.keymap.set(Action::Capture, Some(b));
        key(&mut h, Key::C, Key::C, egui::Modifiers::NONE);
        assert!(h.state().tool == Tool::Select);
        key(&mut h, Key::F2, Key::F2, egui::Modifiers::NONE);
        assert!(h.state().tool == Tool::Image);
    }

    #[test]
    fn note_name_follows_template_and_can_be_typed() {
        if !ok() {
            return;
        }
        let mut h = setup(&["n1"]);
        {
            let app = h.state_mut();
            let t = app.tab().unwrap();
            t.notes.author = "Jane Doe, John Smith".into();
            t.notes.institution = "Seoul National University".into();
            t.notes.year = "2021".into();
            t.notes.title = "Cells".into();
        }
        h.state_mut().settings.obsidian.name_template = "{연도} {저자} - {제목}".into();
        let t = h.state().tab_ref().unwrap();
        assert_eq!(auto_name(&t.notes, &h.state().settings.obsidian.name_template), "2021 Doe & Smith - Cells");
        let md = h.state_mut().tab().unwrap().markdown(&Settings::default());
        assert!(md.contains("## 인용 (Citation)"), "{md}");
        assert!(md.contains("(Doe & Smith, 2021)"));
        assert!(md.contains("@misc{doe2021cells"));
    }

    #[test]
    fn undo_and_redo_annotations() {
        if !ok() {
            return;
        }
        let mut h = setup(&["u1"]);
        h.state_mut().tool = Tool::Highlight;
        h.run_steps(2);
        let page = h.state().debug_page_rect.expect("page drawn");
        drag(&mut h, page.min + Vec2::new(105.0, 145.0), page.min + Vec2::new(400.0, 170.0));
        assert_eq!(count(&h), 1);
        key(&mut h, Key::Z, Key::Z, egui::Modifiers::COMMAND);
        assert_eq!(count(&h), 0, "⌘Z removes it");
        key(&mut h, Key::Z, Key::Z, egui::Modifiers::COMMAND | egui::Modifiers::SHIFT);
        assert_eq!(count(&h), 1, "⌘⇧Z brings it back");
    }

    #[test]
    fn two_documents_side_by_side() {
        if !ok() {
            return;
        }
        let mut h = setup(&["e1", "e2"]);
        assert_eq!(h.state().tabs.len(), 2);
        // Opened with two files: they start side by side.
        assert!(h.state().right.is_some());
        h.get_by_label("나란히 보기").click();
        h.run_steps(4);
        assert!(h.state().right.is_none(), "button turns side by side off");
        h.get_by_label("나란히 보기").click();
        h.run_steps(4);
        assert!(h.state().right.is_some(), "and on again");
        assert_ne!(h.state().right, Some(h.state().left));
        // Each side highlights on its own.
        h.state_mut().tool = Tool::Highlight;
        h.run_steps(2);
        let right = h.state().right.unwrap();
        let page = h.state().tabs[right].debug_first_page.expect("right page drawn");
        drag(&mut h, page.min + Vec2::new(105.0 * page.width() / 612.0, 145.0 * page.width() / 612.0), page.min + Vec2::new(300.0 * page.width() / 612.0, 170.0 * page.width() / 612.0));
        assert_eq!(h.state().tabs[right].notes.annotations.len(), 1);
        assert_eq!(h.state().tabs[h.state().left].notes.annotations.len(), 0);
    }
}

#[cfg(test)]
mod glyph_tests {
    use eframe::egui;
    #[test]
    #[ignore]
    fn glyph_picture() {
        let sample = "漢字研究論文 ①②③ → • ※ … ‘’ “” · ◆ ✓ ⚠ 가나다 한글 — ± × α μ 日本語 中文";
        let mut ready = false;
        let mut h = egui_kittest::Harness::builder().with_size(egui::Vec2::new(900.0, 160.0)).wgpu().build_ui(move |ui| {
            if !ready {
                super::install_fonts(ui.ctx());
                ready = true;
                return;
            }
            egui::CentralPanel::default().show_inside(ui, |ui| {
                ui.label(egui::RichText::new(sample).size(18.0));
                ui.label(egui::RichText::new(sample).size(18.0).family(super::icons::bold()));
                ui.label(egui::RichText::new(sample).size(18.0).monospace());
            });
        });
        h.run_steps(4);
        h.render().unwrap().save("shots/glyphs.png").unwrap();
    }

    #[test]
    #[ignore]
    fn missing_glyphs() {
        let ctx = egui::Context::default();
        super::install_fonts(&ctx);
        let _ = ctx.run_ui(Default::default(), |_| {});
        let sample = "漢字研究論文 ①②③ → • ※ ㈜ … ‘’ “” · ◆ ✓ ⚠ ⓘ ▾ 가나다 ㄱㄴㄷ 한글 ― — ～ ∙ ㆍ ℃ ± × ÷ ≥ α β μ 日本語 中文";
        for fam in [egui::FontFamily::Proportional, super::icons::bold(), egui::FontFamily::Monospace] {
            let id = egui::FontId::new(14.0, fam.clone());
            let missing: String = ctx.fonts_mut(|f| sample.chars().filter(|c| !c.is_whitespace() && !f.has_glyph(&id, *c)).collect());
            println!("{fam:?} missing: {missing}");
        }
    }
}
