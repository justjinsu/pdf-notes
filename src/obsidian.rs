//! Obsidian connection: find the person's vaults, write a document's notes as a
//! Markdown note inside a vault (keeping anything they wrote themselves), copy
//! the PDF and picture selections beside it so page links and images work,
//! and open the note in Obsidian.

use std::path::{Path, PathBuf};

use crate::markdown::{self, Target};
use crate::model::{images_dir, DocNotes, LinkStyle, NoteFormat, ObsidianSettings};

pub const START: &str = "%% highlights:start — 이 부분은 PDF Notes가 자동으로 갱신합니다. 메모는 이 구역 밖에 쓰세요. %%";
pub const START_PREFIX: &str = "%% highlights:start";
pub const END: &str = "%% highlights:end %%";

/// Vault folders Obsidian knows about (from its own obsidian.json).
pub fn known_vaults() -> Vec<String> {
    let file = crate::model::config_base().join("obsidian/obsidian.json");
    let Ok(text) = std::fs::read_to_string(file) else { return Vec::new() };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else { return Vec::new() };
    let mut out: Vec<(i64, bool, String)> = Vec::new();
    if let Some(map) = json.get("vaults").and_then(|v| v.as_object()) {
        for v in map.values() {
            if let Some(p) = v.get("path").and_then(|p| p.as_str()) {
                let ts = v.get("ts").and_then(|t| t.as_i64()).unwrap_or(0);
                let open = v.get("open").and_then(|o| o.as_bool()).unwrap_or(false);
                out.push((ts, open, p.to_string()));
            }
        }
    }
    // The vault open right now first, then the most recently used.
    out.sort_by(|a, b| b.1.cmp(&a.1).then(b.0.cmp(&a.0)));
    out.into_iter().map(|(_, _, p)| p).collect()
}

pub fn vault_name(vault: &str) -> String {
    Path::new(vault).file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()
}

/// File-name-safe version of a title (Obsidian forbids * " \ / < > : | ? # ^ [ ]).
pub fn safe_name(s: &str) -> String {
    let s = crate::pdf::nfc(s);
    let cleaned: String = s
        .chars()
        .map(|c| if "*\"\\/<>:|?#^[]".contains(c) || c.is_control() { ' ' } else { c })
        .collect();
    let cleaned = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let cleaned: String = cleaned.chars().take(120).collect();
    if cleaned.is_empty() { "Untitled".into() } else { cleaned }
}

pub fn note_title(notes: &DocNotes) -> String {
    if !notes.note_name.trim().is_empty() {
        notes.note_name.trim().to_string()
    } else if !notes.title.trim().is_empty() {
        notes.title.trim().to_string()
    } else {
        Path::new(&notes.pdf_path).file_stem().map(|s| crate::pdf::nfc(&s.to_string_lossy())).unwrap_or_else(|| "Untitled".into())
    }
}

pub fn note_path(o: &ObsidianSettings, notes: &DocNotes) -> PathBuf {
    Path::new(&o.vault).join(o.notes_folder.trim_matches('/')).join(format!("{}.md", safe_name(&note_title(notes))))
}

fn yaml_str(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

fn copy_if_changed(from: &Path, to: &Path) -> std::io::Result<()> {
    let same = match (std::fs::metadata(from), std::fs::metadata(to)) {
        (Ok(a), Ok(b)) => a.len() == b.len() && a.modified().ok() <= b.modified().ok(),
        _ => false,
    };
    if !same {
        if let Some(d) = to.parent() {
            std::fs::create_dir_all(d)?;
        }
        std::fs::copy(from, to)?;
    }
    Ok(())
}

pub struct SyncResult {
    pub note: PathBuf,
    pub count: usize,
    /// The memo as it now stands in both places (it may have come from Obsidian).
    pub memo: String,
}

pub const MEMO_START: &str = "%% memo:start — 여기는 PDF Notes의 ‘내 메모’와 함께 바뀌어요. 자유롭게 쓰세요. %%";
pub const MEMO_START_PREFIX: &str = "%% memo:start";
pub const MEMO_END: &str = "%% memo:end %%";

/// The memo section of a note, if it has one.
pub fn read_memo(text: &str) -> Option<String> {
    let s = text.find(MEMO_START_PREFIX)?;
    let after_start = s + text[s..].find('\n')? + 1;
    let e = after_start + text[after_start..].find(MEMO_END)?;
    Some(text[after_start..e].trim_matches('\n').to_string())
}

/// Three-way: `app` and `file` against what both had last time (`last`).
/// One side changed → take it; both changed → keep both.
pub fn merge_memo(app: &str, file: Option<&str>, last: &str) -> String {
    let Some(file) = file else { return app.to_string() };
    let (a, f, l) = (app.trim(), file.trim(), last.trim());
    if f == l || f == a {
        a.to_string()
    } else if a == l {
        f.to_string()
    } else if f.contains(a) {
        f.to_string()
    } else if a.contains(f) {
        a.to_string()
    } else {
        format!("{f}\n\n{a}")
    }
}

fn memo_block(memo: &str) -> String {
    format!("{MEMO_START}\n{}\n{MEMO_END}\n", memo.trim_end())
}

/// Put `memo` into the note's memo section (adding the section if missing).
fn with_memo(text: &str, memo: &str) -> String {
    let block = memo_block(memo);
    match (text.find(MEMO_START_PREFIX), text.find(MEMO_END)) {
        (Some(s), Some(e)) if e > s => {
            let line_start = text[..s].rfind('\n').map(|i| i + 1).unwrap_or(0);
            let mut after = e + MEMO_END.len();
            if text[after..].starts_with('\n') {
                after += 1;
            }
            format!("{}{}{}", &text[..line_start], block, &text[after..])
        }
        _ => {
            // After the highlights block, under its own heading.
            let insert_at = text.find(END).map(|e| {
                let a = e + END.len();
                if text[a..].starts_with('\n') { a + 1 } else { a }
            });
            let section = format!("\n## 내 메모\n{block}");
            match insert_at {
                Some(i) => format!("{}{}{}", &text[..i], section, &text[i..]),
                None => format!("{}\n{}", text.trim_end(), section),
            }
        }
    }
}

/// Write (or update) the note for `notes` in the vault.
pub fn sync(
    notes: &DocNotes,
    format: &NoteFormat,
    o: &ObsidianSettings,
    page_label: &dyn Fn(usize) -> String,
) -> Result<SyncResult, String> {
    let vault = Path::new(&o.vault);
    if o.vault.trim().is_empty() {
        return Err("Obsidian 보관함(vault)을 먼저 골라 주세요.".into());
    }
    if !vault.is_dir() {
        return Err(format!("보관함 폴더를 찾을 수 없어요: {}", o.vault));
    }
    let attach_rel = o.attachments_folder.trim_matches('/').to_string();
    let attach_dir = vault.join(&attach_rel);
    let pdf = Path::new(&notes.pdf_path);
    let pdf_name = pdf.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "document.pdf".into());

    // The PDF itself, so "[[paper.pdf#page=3]]" opens at that page in Obsidian.
    let mut settings = o.clone();
    let pdf_link = if o.copy_pdf && pdf.is_file() {
        copy_if_changed(pdf, &attach_dir.join(&pdf_name)).map_err(|e| format!("PDF를 보관함에 복사하지 못했어요: {e}"))?;
        if attach_rel.is_empty() { pdf_name.clone() } else { format!("{attach_rel}/{pdf_name}") }
    } else {
        if settings.link_style == LinkStyle::ObsidianPdf {
            settings.link_style = LinkStyle::FileLink;
        }
        notes.pdf_path.clone()
    };

    // Picture selections.
    for a in &notes.annotations {
        if let Some(file) = &a.image_file {
            let src = images_dir().join(file);
            if src.is_file() {
                copy_if_changed(&src, &attach_dir.join(file)).map_err(|e| format!("그림을 복사하지 못했어요: {e}"))?;
            }
        }
    }
    let image_prefix = if attach_rel.is_empty() { String::new() } else { format!("{attach_rel}/") };

    let target = Target::Obsidian { settings: &settings, pdf_link: &pdf_link, image_prefix: &image_prefix };
    let md = markdown::build(notes, format, &target, page_label);

    let mut tags: Vec<String> = o
        .extra_tags
        .split([',', ' '])
        .map(|t| t.trim().trim_start_matches('#').to_string())
        .filter(|t| !t.is_empty())
        .collect();
    for t in md.tags {
        if !tags.contains(&t) {
            tags.push(t);
        }
    }

    let block = format!("{START}\n{}\n{END}\n", md.body.trim_end());
    let path = note_path(o, notes);
    let today = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();
    let tag_line = format!("tags: [{}]", tags.iter().map(|t| yaml_str(t)).collect::<Vec<_>>().join(", "));
    let source_line = if o.copy_pdf {
        format!("source: {}", yaml_str(&format!("[[{pdf_link}]]")))
    } else {
        format!("source: {}", yaml_str(&format!("file://{}", notes.pdf_path)))
    };

    // The note name changed since the last sync: move the old note (with the
    // person's own writing in it) instead of starting a second one.
    if !notes.synced_note.is_empty() {
        let old = PathBuf::from(&notes.synced_note);
        if old != path && old.is_file() && !path.exists() && old.starts_with(vault) {
            if let Some(d) = path.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            let _ = std::fs::rename(&old, &path);
        }
    }
    let existing = std::fs::read_to_string(&path).ok();
    let memo = merge_memo(&notes.memo, existing.as_deref().and_then(read_memo).as_deref(), &notes.memo_synced);
    let content = match existing {
        Some(existing) => with_memo(&update_existing(&existing, &block, &tag_line, &today, notes.annotations.len(), o.frontmatter), &memo),
        None => {
            let mut s = String::new();
            if o.frontmatter {
                s.push_str("---\n");
                let doc_title = if notes.title.trim().is_empty() { note_title(notes) } else { notes.title.trim().to_string() };
                s.push_str(&format!("title: {}\n", yaml_str(&doc_title)));
                if !notes.author.trim().is_empty() {
                    s.push_str(&format!("author: {}\n", yaml_str(notes.author.trim())));
                }
                if !notes.institution.trim().is_empty() {
                    s.push_str(&format!("institution: {}\n", yaml_str(notes.institution.trim())));
                }
                if !notes.year.trim().is_empty() {
                    s.push_str(&format!("year: {}\n", notes.year.trim()));
                }
                if !notes.journal.trim().is_empty() {
                    s.push_str(&format!("journal: {}\n", yaml_str(notes.journal.trim())));
                }
                if !notes.doi.trim().is_empty() {
                    s.push_str(&format!("doi: {}\n", yaml_str(notes.doi.trim())));
                }
                s.push_str(&format!("citekey: {}\n", yaml_str(&crate::citation::Work::from_notes(notes).citekey())));
                s.push_str(&source_line);
                s.push('\n');
                s.push_str(&tag_line);
                s.push('\n');
                s.push_str(&format!("annotations: {}\n", notes.annotations.len()));
                s.push_str(&format!("created: {}\n", today));
                s.push_str(&format!("updated: {}\n", today));
                s.push_str("---\n");
            }
            s.push_str(&block);
            s.push_str("\n## 내 메모\n");
            s.push_str(&memo_block(&memo));
            s
        }
    };
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).map_err(|e| format!("폴더를 만들지 못했어요: {e}"))?;
    }
    std::fs::write(&path, content).map_err(|e| format!("노트를 쓰지 못했어요: {e}"))?;
    Ok(SyncResult { note: path, count: notes.annotations.len(), memo })
}

/// Replace only the managed block (and a few frontmatter lines we own) in a
/// note the person may have edited.
pub fn update_existing(existing: &str, block: &str, tag_line: &str, today: &str, count: usize, frontmatter: bool) -> String {
    let mut text = existing.to_string();

    // Frontmatter lines we own.
    if frontmatter && text.starts_with("---\n") {
        if let Some(end) = text[4..].find("\n---").map(|i| i + 4) {
            let fm = &text[4..end];
            let mut lines: Vec<String> = Vec::new();
            for line in fm.lines() {
                if line.starts_with("updated:") {
                    lines.push(format!("updated: {today}"));
                } else if line.starts_with("annotations:") {
                    lines.push(format!("annotations: {count}"));
                } else if line.starts_with("tags: [") {
                    // Keep tags the person added, add ours.
                    let mut have: Vec<String> = line["tags: [".len()..]
                        .trim_end_matches(']')
                        .split(',')
                        .map(|t| t.trim().trim_matches('"').to_string())
                        .filter(|t| !t.is_empty())
                        .collect();
                    for t in tag_line["tags: [".len()..].trim_end_matches(']').split(',') {
                        let t = t.trim().trim_matches('"').to_string();
                        if !t.is_empty() && !have.contains(&t) {
                            have.push(t);
                        }
                    }
                    lines.push(format!("tags: [{}]", have.iter().map(|t| format!("\"{t}\"")).collect::<Vec<_>>().join(", ")));
                } else {
                    lines.push(line.to_string());
                }
            }
            text = format!("---\n{}{}", lines.join("\n"), &text[end..]);
        }
    }

    match (text.find(START_PREFIX), text.find(END)) {
        (Some(s), Some(e)) if e > s => {
            let line_start = text[..s].rfind('\n').map(|i| i + 1).unwrap_or(0);
            let after = e + END.len();
            let after = if text[after..].starts_with('\n') { after + 1 } else { after };
            format!("{}{}{}", &text[..line_start], block, &text[after..])
        }
        _ => {
            let mut t = text;
            if !t.ends_with('\n') {
                t.push('\n');
            }
            t.push('\n');
            t.push_str(block);
            t
        }
    }
}

/// Open a note (any path inside a vault) in Obsidian.
pub fn open_in_obsidian(note: &Path) -> Result<(), String> {
    let url = format!("obsidian://open?path={}", encode_component(&note.to_string_lossy()));
    crate::model::open_url(&url)
}

pub fn encode_component(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memo_goes_both_ways() {
        // Only Obsidian changed → app takes it; only the app changed → file takes it.
        assert_eq!(merge_memo("a", Some("a b"), "a"), "a b");
        assert_eq!(merge_memo("a c", Some("a"), "a"), "a c");
        assert_eq!(merge_memo("x", Some("y"), "z"), "y\n\nx");
        let note = format!("---\n---\n{START}\nhl\n{END}\n\n## 내 생각\nmine\n");
        let with = with_memo(&note, "first memo");
        assert!(with.contains("## 내 메모\n%% memo:start"));
        assert_eq!(read_memo(&with).as_deref(), Some("first memo"));
        let again = with_memo(&with, "second\nline");
        assert_eq!(read_memo(&again).as_deref(), Some("second\nline"));
        assert!(again.contains("mine"));
        assert_eq!(again.matches(MEMO_END).count(), 1);
    }

    #[test]
    fn keeps_text_outside_block() {
        let old = format!("---\ntitle: \"A\"\ntags: [\"mine\"]\nupdated: x\n---\nintro\n{START}\nold\n{END}\n\n## 내 생각\nmy thoughts\n");
        let new_block = format!("{START}\nnew\n{END}\n");
        let out = update_existing(&old, &new_block, "tags: [\"highlights\"]", "today", 3, true);
        assert!(out.contains("my thoughts"));
        assert!(out.contains("intro"));
        assert!(out.contains("\nnew\n"));
        assert!(!out.contains("\nold\n"));
        assert!(out.contains("\"mine\", \"highlights\""));
        assert!(out.contains("updated: today"));
    }
}
