//! Turns a document's notes into Markdown, following the layout the original
//! writes: a metadata header, one `####` heading per page
//! with a link back to that page, each annotation as a blockquote wrapped in
//! the type's start/end tags, and its comment underneath.

use crate::model::{hashtags, Annotation, DocNotes, Kind, LinkStyle, NoteFormat, ObsidianSettings, COLOR_NAMES_EN};

/// Where the Markdown is going: a plain `.md` file, or an Obsidian vault.
pub enum Target<'a> {
    Plain { pdf_path: &'a str },
    Obsidian { settings: &'a ObsidianSettings, pdf_link: &'a str, image_prefix: &'a str },
}

pub struct Output {
    pub body: String,
    pub tags: Vec<String>,
}

fn url_escape(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => out.push(b as char),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

fn wrap(text: &str, width: usize, prefix: &str) -> String {
    let mut lines = Vec::new();
    let mut cur = String::new();
    for word in text.split_whitespace() {
        if !cur.is_empty() && cur.chars().count() + 1 + word.chars().count() > width {
            lines.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(word);
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines.into_iter().map(|l| format!("{prefix}{l}")).collect::<Vec<_>>().join("\n")
}

fn page_heading(f: &NoteFormat, target: &Target, page_index: usize, label: &str) -> String {
    let title = format!("Page {label}");
    if !f.use_page_links {
        return format!("#### {title}");
    }
    match target {
        Target::Plain { pdf_path } => {
            format!("#### [{title}](file://{}#page={})", url_escape(pdf_path), page_index + 1)
        }
        Target::Obsidian { settings, pdf_link, .. } => match settings.link_style {
            LinkStyle::ObsidianPdf => format!("#### [[{}#page={}|{title}]]", pdf_link, page_index + 1),
            LinkStyle::FileLink => format!("#### [{title}](file://{}#page={})", url_escape(pdf_link), page_index + 1),
            LinkStyle::None => format!("#### {title}"),
        },
    }
}

fn info_line(f: &NoteFormat, a: &Annotation, label: &str) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    if f.include_category {
        if let Some(c) = f.categories.get(a.color) {
            parts.push(c.clone());
        }
    }
    if f.include_color {
        parts.push(COLOR_NAMES_EN.get(a.color).unwrap_or(&"Other").to_string());
    }
    if f.include_type {
        parts.push(a.kind.type_name().to_string());
    }
    if f.include_page {
        parts.push(format!("Page {label}"));
    }
    if f.include_author {
        let who = if a.author.is_empty() { f.user_name.clone() } else { a.author.clone() };
        if !who.is_empty() {
            parts.push(who);
        }
    }
    if f.include_date && !a.date.is_empty() {
        parts.push(a.date.clone());
    }
    if parts.is_empty() { None } else { Some(format!("##### {}:", parts.join(", "))) }
}

fn annotation_block(f: &NoteFormat, target: &Target, a: &Annotation, label: &str) -> String {
    let mut out = String::new();
    if let Some(info) = info_line(f, a, label) {
        out.push_str(&info);
        out.push_str("\n\n");
    }
    let callouts = matches!(target, Target::Obsidian { settings, .. } if settings.callouts);
    let (start, end) = match a.kind {
        Kind::Highlight => (f.highlight_start.as_str(), f.highlight_end.as_str()),
        Kind::Underline => (f.underline_start.as_str(), f.underline_end.as_str()),
        Kind::Strike => (f.strike_start.as_str(), f.strike_end.as_str()),
        _ => ("", ""),
    };
    match a.kind {
        Kind::Highlight | Kind::Underline | Kind::Strike => {
            let inner = format!("{start}{}{end}", a.text.trim());
            if callouts {
                let cat = f.categories.get(a.color).cloned().unwrap_or_default();
                out.push_str(&format!("> [!quote] {cat} · p.{label}\n"));
                out.push_str(&if f.use_text_wrapping { wrap(&inner, f.page_width, "> ") } else { format!("> {inner}") });
            } else if f.use_blockquotes {
                out.push_str(&if f.use_text_wrapping { wrap(&inner, f.page_width, "> ") } else { format!("> {inner}") });
            } else {
                let q = format!("{}{}{}", f.quote_start, inner, f.quote_end);
                out.push_str(&if f.use_text_wrapping { wrap(&q, f.page_width, "") } else { q });
            }
            out.push('\n');
        }
        Kind::Image => {
            if let Some(file) = &a.image_file {
                match target {
                    Target::Obsidian { image_prefix, .. } => out.push_str(&format!("![[{}{}]]\n", image_prefix, file)),
                    Target::Plain { .. } => out.push_str(&format!("![{}](assets/{})\n", a.text.trim(), file)),
                }
            }
        }
        Kind::Note => {}
    }
    let comment = a.comment.trim();
    if !comment.is_empty() {
        if a.kind != Kind::Note {
            out.push('\n');
        }
        out.push_str(&if f.use_text_wrapping { wrap(comment, f.page_width, "") } else { comment.to_string() });
        out.push('\n');
    }
    out
}

/// Build the notes. `page_label(i)` gives the printed label of page `i`.
pub fn build(notes: &DocNotes, f: &NoteFormat, target: &Target, page_label: &dyn Fn(usize) -> String) -> Output {
    let mut out = String::new();
    let mut tags: Vec<String> = Vec::new();

    // Metadata header: "# Title", "## Author", "### [doi](link)".
    let heading_name = matches!(target, Target::Obsidian { settings, .. } if settings.heading_is_note_name) && !notes.note_name.trim().is_empty();
    let title = if heading_name {
        notes.note_name.trim().to_string()
    } else if notes.title.trim().is_empty() {
        std::path::Path::new(&notes.pdf_path).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()
    } else {
        notes.title.trim().to_string()
    };
    if f.use_atx_header {
        out.push_str(&format!("# {title}\n"));
        if !notes.author.trim().is_empty() {
            out.push_str(&format!("## {}\n", notes.author.trim()));
        }
    } else {
        out.push_str(&format!("{title}\n{}\n", "=".repeat(30)));
        if !notes.author.trim().is_empty() {
            out.push_str(&format!("{}\n{}\n", notes.author.trim(), "-".repeat(30)));
        }
    }
    if !notes.doi.trim().is_empty() {
        let doi = notes.doi.trim();
        out.push_str(&format!("### [{doi}](https://doi.org/{doi})\n"));
    }

    let sorted = notes.sorted();
    let groups: Vec<(Option<usize>, Vec<&Annotation>)> = if f.sort_by_color {
        (0..8)
            .map(|c| (Some(c), sorted.iter().copied().filter(|a| a.color == c).collect::<Vec<_>>()))
            .filter(|(_, v)| !v.is_empty())
            .collect()
    } else {
        vec![(None, sorted)]
    };

    for (color, list) in groups {
        if let Some(c) = color {
            out.push_str(&format!("\n### {}\n", f.categories.get(c).cloned().unwrap_or_default()));
        }
        let mut last_page: Option<usize> = None;
        for (i, a) in list.iter().enumerate() {
            let label = page_label(a.page);
            if last_page != Some(a.page) {
                out.push('\n');
                out.push_str(&page_heading(f, target, a.page, &label));
                out.push('\n');
                last_page = Some(a.page);
            } else if f.include_spacer && i > 0 {
                out.push_str("\n***\n");
            }
            out.push('\n');
            out.push_str(&annotation_block(f, target, a, &label));
            for t in hashtags(&a.comment, f.lower_case_tags) {
                if !tags.contains(&t) {
                    tags.push(t);
                }
            }
        }
    }
    out.push_str(&crate::citation::section(notes, &f.citation));
    Output { body: out, tags }
}
