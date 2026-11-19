//! Notes data: annotations, colours, note-format options and Obsidian settings,
//! plus where they are saved on disk.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct ViewRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl ViewRect {
    pub fn from_points(a: (f32, f32), b: (f32, f32)) -> Self {
        ViewRect { x: a.0.min(b.0), y: a.1.min(b.1), w: (a.0 - b.0).abs(), h: (a.1 - b.1).abs() }
    }
    pub fn contains(&self, p: (f32, f32)) -> bool {
        p.0 >= self.x && p.0 <= self.x + self.w && p.1 >= self.y && p.1 <= self.y + self.h
    }
    pub fn overlap_ratio(&self, o: &ViewRect) -> f32 {
        let x0 = self.x.max(o.x);
        let y0 = self.y.max(o.y);
        let x1 = (self.x + self.w).min(o.x + o.w);
        let y1 = (self.y + self.h).min(o.y + o.h);
        if x1 <= x0 || y1 <= y0 {
            return 0.0;
        }
        let inter = (x1 - x0) * (y1 - y0);
        let smaller = (self.w * self.h).min(o.w * o.h).max(0.01);
        inter / smaller
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Hash)]
pub enum Kind {
    Highlight,
    Underline,
    Strike,
    Note,
    Image,
}

impl Kind {
    /// The type names the original writes ("Highlight", "Underline", "StrikeOut",
    /// "Sticky Note", "Image Selection").
    pub fn type_name(self) -> &'static str {
        match self {
            Kind::Highlight => "Highlight",
            Kind::Underline => "Underline",
            Kind::Strike => "StrikeOut",
            Kind::Note => "Sticky Note",
            Kind::Image => "Image Selection",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Kind::Highlight => "형광펜",
            Kind::Underline => "밑줄",
            Kind::Strike => "취소선",
            Kind::Note => "메모",
            Kind::Image => "이미지 선택",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Annotation {
    pub id: u64,
    pub kind: Kind,
    pub page: usize,
    pub color: usize,
    pub rects: Vec<ViewRect>,
    pub text: String,
    #[serde(default)]
    pub comment: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub date: String,
    /// File name of the cropped picture for image selections.
    #[serde(default)]
    pub image_file: Option<String>,
}

impl Annotation {
    pub fn new(kind: Kind, page: usize, color: usize, rects: Vec<ViewRect>, text: String) -> Self {
        let now = chrono::Local::now();
        Annotation {
            id: (now.timestamp_nanos_opt().unwrap_or(0) as u64) ^ ((page as u64) << 48) ^ rand_bits(),
            kind,
            page,
            color,
            rects,
            text,
            comment: String::new(),
            author: String::new(),
            date: now.format("%Y-%m-%d").to_string(),
            image_file: None,
        }
    }
    pub fn top(&self) -> f32 {
        self.rects.iter().map(|r| r.y).fold(f32::MAX, f32::min)
    }
    pub fn left(&self) -> f32 {
        self.rects.iter().map(|r| r.x).fold(f32::MAX, f32::min)
    }
    pub fn hit(&self, p: (f32, f32)) -> bool {
        self.rects.iter().any(|r| {
            let grow = if self.kind == Kind::Note { 4.0 } else { 1.5 };
            ViewRect { x: r.x - grow, y: r.y - grow, w: r.w + grow * 2.0, h: r.h + grow * 2.0 }.contains(p)
        })
    }
}

fn rand_bits() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(1);
    COUNTER.fetch_add(0x9E37_79B9, Ordering::Relaxed)
}

/// The eight annotation colours, in the order the original shows them
/// (popover: red, orange, yellow, green, blue, pink, purple, grey).
/// RGB values are estimated from the original's guide pictures.
pub const PALETTE: [[u8; 3]; 8] = [
    [242, 107, 107],
    [245, 165, 91],
    [250, 236, 70],
    [60, 212, 90],
    [63, 200, 232],
    [242, 107, 220],
    [178, 112, 217],
    [173, 173, 173],
];

/// Names the original uses in exports (and the CSS classes red, orange, ...).
pub const COLOR_NAMES_EN: [&str; 8] = ["Red", "Orange", "Yellow", "Green", "Blue", "Pink", "Purple", "Other"];
pub const COLOR_NAMES_KO: [&str; 8] = ["빨강", "주황", "노랑", "초록", "파랑", "분홍", "보라", "회색"];

pub fn nearest_color(rgb: [i32; 3]) -> usize {
    let mut best = 2;
    let mut best_d = i32::MAX;
    for (i, c) in PALETTE.iter().enumerate() {
        let d = (0..3).map(|k| (c[k] as i32 - rgb[k]).pow(2)).sum::<i32>();
        if d < best_d {
            best_d = d;
            best = i;
        }
    }
    best
}

/// Note format options. Names and defaults follow the original's
/// DefaultPreferences.plist.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct NoteFormat {
    pub use_atx_header: bool,
    pub use_page_links: bool,
    pub include_page: bool,
    pub include_color: bool,
    pub include_author: bool,
    pub include_date: bool,
    pub include_type: bool,
    pub include_category: bool,
    pub include_spacer: bool,
    pub sort_by_color: bool,
    pub lower_case_tags: bool,
    pub use_text_wrapping: bool,
    pub page_width: usize,
    pub highlight_start: String,
    pub highlight_end: String,
    pub underline_start: String,
    pub underline_end: String,
    pub strike_start: String,
    pub strike_end: String,
    pub quote_start: String,
    pub quote_end: String,
    pub use_blockquotes: bool,
    pub user_name: String,
    /// The "인용" section at the end of the notes.
    pub citation: crate::citation::CitationOptions,
    /// Colour category names (user-editable), one per colour.
    pub categories: Vec<String>,
}

impl Default for NoteFormat {
    fn default() -> Self {
        NoteFormat {
            use_atx_header: true,
            use_page_links: true,
            include_page: false,
            include_color: false,
            include_author: false,
            include_date: false,
            include_type: false,
            include_category: false,
            include_spacer: false,
            sort_by_color: false,
            lower_case_tags: true,
            use_text_wrapping: false,
            page_width: 64,
            highlight_start: String::new(),
            highlight_end: String::new(),
            underline_start: "**".into(),
            underline_end: "**".into(),
            strike_start: "<del>".into(),
            strike_end: "</del>".into(),
            quote_start: "\"".into(),
            quote_end: "\"".into(),
            use_blockquotes: true,
            user_name: String::new(),
            citation: Default::default(),
            categories: COLOR_NAMES_KO.iter().map(|s| s.to_string()).collect(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LinkStyle {
    /// `[[paper.pdf#page=3|Page 3]]` — opens in Obsidian's own PDF viewer.
    ObsidianPdf,
    /// `[Page 3](file:///.../paper.pdf)` — opens the PDF in this app/Preview.
    FileLink,
    /// `#### Page 3` without a link.
    None,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct ObsidianSettings {
    pub vault: String,
    pub notes_folder: String,
    pub attachments_folder: String,
    pub auto_sync: bool,
    pub copy_pdf: bool,
    pub link_style: LinkStyle,
    pub frontmatter: bool,
    pub callouts: bool,
    pub extra_tags: String,
    /// Note name pattern: {저자} {저자전체} {기관} {연도} {제목} {파일}.
    pub name_template: String,
    /// Use the note name (instead of the document title) as the note's # heading.
    pub heading_is_note_name: bool,
}

impl Default for ObsidianSettings {
    fn default() -> Self {
        ObsidianSettings {
            vault: String::new(),
            notes_folder: "Highlights".into(),
            attachments_folder: "Highlights/attachments".into(),
            auto_sync: true,
            copy_pdf: true,
            link_style: LinkStyle::ObsidianPdf,
            frontmatter: true,
            callouts: false,
            extra_tags: "highlights".into(),
            name_template: crate::naming::DEFAULT_TEMPLATE.into(),
            heading_is_note_name: false,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub format: NoteFormat,
    pub obsidian: ObsidianSettings,
    pub save_into_pdf: bool,
    pub dark: bool,
    /// Light/dark follows the Mac's (or phone's) setting.
    pub follow_system: bool,
    pub dark_pdf: bool,
    pub default_color: usize,
    pub recent: Vec<String>,
    pub keymap: crate::shortcuts::Keymap,
    pub ai: crate::ai::AiSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            format: NoteFormat::default(),
            obsidian: ObsidianSettings::default(),
            save_into_pdf: true,
            dark: false,
            follow_system: true,
            dark_pdf: true,
            default_color: 2,
            recent: Vec::new(),
            keymap: Default::default(),
            ai: Default::default(),
        }
    }
}

/// The person's home folder (`HOME`; on Windows `USERPROFILE`).
pub fn home_dir() -> PathBuf {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(var).or_else(|| std::env::var_os("HOME")).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}

/// Where programs keep their settings: `~/Library/Application Support` on a
/// Mac, `%APPDATA%` on Windows, `~/.config` elsewhere.
pub fn config_base() -> PathBuf {
    if cfg!(target_os = "macos") || cfg!(target_os = "ios") {
        home_dir().join("Library/Application Support")
    } else if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| home_dir().join("AppData/Roaming"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home_dir().join(".config"))
    }
}

/// Hand a link (https://, obsidian://…) to whatever program opens it.
pub fn open_url(url: &str) -> Result<(), String> {
    let mut cmd = if cfg!(windows) {
        // Not `cmd /c start`: that would read `%` and `&` in the link its own way.
        let mut c = std::process::Command::new("rundll32");
        c.args(["url.dll,FileProtocolHandler", url]);
        c
    } else {
        let mut c = std::process::Command::new(if cfg!(target_os = "macos") { "open" } else { "xdg-open" });
        c.arg(url);
        c
    };
    no_console(&mut cmd);
    cmd.spawn().map(|_| ()).map_err(|e| e.to_string())
}

/// On Windows, starting a command-line program from a window app flashes a
/// black console window unless we ask for none.
pub fn no_console(cmd: &mut std::process::Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    #[cfg(not(windows))]
    let _ = cmd;
}

pub fn data_dir() -> PathBuf {
    config_base().join("PDF Notes")
}

impl Settings {
    pub fn load() -> Settings {
        std::fs::read_to_string(data_dir().join("settings.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }
    pub fn save(&self) {
        let dir = data_dir();
        let _ = std::fs::create_dir_all(&dir);
        if let Ok(s) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(dir.join("settings.json"), s);
        }
    }
    pub fn remember(&mut self, path: &Path) {
        let p = path.to_string_lossy().to_string();
        self.recent.retain(|r| r != &p);
        self.recent.insert(0, p);
        self.recent.truncate(10);
    }
}

/// Our own record of a PDF's notes, kept beside the app data so comments and
/// image selections survive even when the PDF itself is read-only.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DocNotes {
    pub pdf_path: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub doi: String,
    #[serde(default)]
    pub institution: String,
    #[serde(default)]
    pub year: String,
    /// "저자, 기관 (연도)" — the Obsidian note's name. Made automatically.
    #[serde(default)]
    pub note_name: String,
    /// Where the Obsidian note was last written (to follow renames).
    #[serde(default)]
    pub synced_note: String,
    /// Publication details for citations (from CrossRef or typed in).
    #[serde(default)]
    pub authors: Vec<crate::citation::Person>,
    #[serde(default)]
    pub journal: String,
    #[serde(default)]
    pub volume: String,
    #[serde(default)]
    pub issue: String,
    #[serde(default)]
    pub pages: String,
    #[serde(default)]
    pub publisher: String,
    /// The note name was typed by the person (don't re-make it automatically).
    #[serde(default)]
    pub note_name_custom: bool,
    /// The person's own free-form memo about this document.
    #[serde(default)]
    pub memo: String,
    /// The memo as it was at the last exchange with Obsidian.
    #[serde(default)]
    pub memo_synced: String,
    pub annotations: Vec<Annotation>,
}

fn notes_file(pdf: &Path) -> PathBuf {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    pdf.to_string_lossy().hash(&mut h);
    data_dir().join("notes").join(format!("{:016x}.json", h.finish()))
}

pub fn images_dir() -> PathBuf {
    data_dir().join("images")
}

impl DocNotes {
    pub fn load(pdf: &Path) -> Option<DocNotes> {
        let s = std::fs::read_to_string(notes_file(pdf)).ok()?;
        serde_json::from_str(&s).ok()
    }
    pub fn save(&self, pdf: &Path) {
        let f = notes_file(pdf);
        if let Some(d) = f.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        if let Ok(s) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(f, s);
        }
    }

    /// Reading order: page, then top to bottom, then left to right.
    pub fn sorted(&self) -> Vec<&Annotation> {
        let mut v: Vec<&Annotation> = self.annotations.iter().collect();
        v.sort_by(|a, b| {
            a.page
                .cmp(&b.page)
                .then((a.top() / 4.0).round().partial_cmp(&(b.top() / 4.0).round()).unwrap_or(std::cmp::Ordering::Equal))
                .then(a.left().partial_cmp(&b.left()).unwrap_or(std::cmp::Ordering::Equal))
        });
        v
    }
}

/// `#tag` words in a comment (letters, digits, `_`, `-`, `/`; any script).
pub fn hashtags(comment: &str, lower: bool) -> Vec<String> {
    let mut out = Vec::new();
    for word in comment.split_whitespace() {
        if let Some(rest) = word.strip_prefix('#') {
            let tag: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '-' || *c == '/')
                .collect();
            if !tag.is_empty() && !tag.chars().all(|c| c.is_ascii_digit()) {
                let tag = if lower { tag.to_lowercase() } else { tag };
                if !out.contains(&tag) {
                    out.push(tag);
                }
            }
        }
    }
    out
}
