//! Automatic note name: "저자, 기관 (연도)" — first author(s), institution and
//! year — guessed from the PDF's own details and the text of its first page.

const INSTITUTION_WORDS: [(&str, u8); 30] = [
    ("대학교", 10),
    ("university", 10),
    ("universität", 10),
    ("université", 10),
    ("universidad", 10),
    ("연구원", 9),
    ("연구소", 9),
    ("institute", 9),
    ("institut", 9),
    ("college", 8),
    ("hospital", 8),
    ("병원", 8),
    ("대학", 7),
    ("school of", 6),
    ("center", 6),
    ("centre", 6),
    ("laboratory", 6),
    ("laboratories", 6),
    ("academy", 6),
    ("ministry", 6),
    ("agency", 6),
    ("foundation", 5),
    ("corporation", 5),
    ("inc.", 4),
    ("ltd", 4),
    ("공사", 5),
    ("재단", 5),
    ("기관", 4),
    ("department", 2),
    ("학과", 2),
];

fn is_hangul(c: char) -> bool {
    ('\u{AC00}'..='\u{D7A3}').contains(&c)
}

fn strip_marks(s: &str) -> String {
    s.chars()
        .filter(|c| !matches!(c, '*' | '†' | '‡' | '§' | '¶' | '#') && !c.is_ascii_digit() && !('\u{2070}'..='\u{209F}').contains(c) && !matches!(c, '¹' | '²' | '³'))
        .collect::<String>()
        .trim()
        .trim_matches(|c: char| c == ',' || c == ';' || c.is_whitespace())
        .to_string()
}

/// Split an author string into names.
pub fn split_authors(s: &str) -> Vec<String> {
    let s = s.replace(" and ", ",").replace(" & ", ",").replace('·', ",").replace('、', ",");
    let parts: Vec<&str> = if s.contains(';') { s.split(';').collect() } else { s.split(',').collect() };
    parts
        .into_iter()
        .map(strip_marks)
        .filter(|p| !p.is_empty() && p.chars().any(|c| c.is_alphabetic()))
        .collect()
}

fn surname(name: &str) -> String {
    let name = name.trim();
    if name.chars().any(is_hangul) {
        return name.split_whitespace().collect::<Vec<_>>().join("");
    }
    // "Last, First" was already split away; take the last word ("Jane Q. Doe" -> "Doe").
    name.split_whitespace().last().unwrap_or(name).trim_matches('.').to_string()
}

/// "Kim", "Kim & Lee", "Kim et al.", "김철수 외".
pub fn format_authors(names: &[String]) -> String {
    match names.len() {
        0 => String::new(),
        1 => surname(&names[0]),
        2 => {
            if names[0].chars().any(is_hangul) {
                format!("{}·{}", surname(&names[0]), surname(&names[1]))
            } else {
                format!("{} & {}", surname(&names[0]), surname(&names[1]))
            }
        }
        _ => {
            if names[0].chars().any(is_hangul) {
                format!("{} 외", surname(&names[0]))
            } else {
                format!("{} et al.", surname(&names[0]))
            }
        }
    }
}

fn looks_like_name(word: &str) -> bool {
    let w = word.trim_matches(|c: char| !c.is_alphabetic() && c != '.' && c != '-');
    if w.is_empty() {
        return false;
    }
    if w.chars().all(is_hangul) {
        return (2..=4).contains(&w.chars().count());
    }
    let first = w.chars().next().unwrap();
    first.is_uppercase() && (w.chars().count() <= 2 || w.chars().skip(1).any(|c| c.is_lowercase()) || w.ends_with('.'))
}

/// Guess the author line among the first lines of page 1.
pub fn authors_from_text(first_page: &str) -> Option<String> {
    let lines: Vec<&str> = first_page.lines().map(|l| l.trim()).filter(|l| !l.is_empty()).take(30).collect();
    for (i, line) in lines.iter().enumerate() {
        if i == 0 || line.chars().count() > 220 {
            continue;
        }
        let lower = line.to_lowercase();
        if INSTITUTION_WORDS.iter().any(|(w, _)| lower.contains(w))
            || ["abstract", "keywords", "doi", "http", "journal", "vol", "received", "©", "copyright", "초록", "요약"].iter().any(|w| lower.contains(w))
        {
            continue;
        }
        let words: Vec<&str> = line.split_whitespace().collect();
        if words.len() < 2 || words.len() > 30 {
            continue;
        }
        let namey = words.iter().filter(|w| looks_like_name(w)).count();
        let has_sep = line.contains(',') || line.contains(" and ") || line.contains('·') || line.contains('&');
        if namey * 10 >= words.len() * 8 && (has_sep || words.len() <= 4) && !line.ends_with('.') {
            let names = split_authors(line);
            if !names.is_empty() && names.iter().all(|n| n.split_whitespace().count() <= 5) {
                return Some(line.to_string());
            }
        }
    }
    None
}

/// The most specific organisation named on page 1 ("Seoul National University").
pub fn institution_from_text(first_page: &str) -> Option<String> {
    let mut best: Option<(u8, usize, String)> = None;
    for (li, line) in first_page.lines().take(80).enumerate() {
        for seg in line.split([',', ';', '|']) {
            let seg = strip_marks(seg);
            let lower = seg.to_lowercase();
            if seg.chars().count() < 3 || seg.chars().count() > 90 {
                continue;
            }
            if let Some((_, score)) = INSTITUTION_WORDS.iter().filter(|(w, _)| lower.contains(w)).max_by_key(|(_, s)| *s) {
                let better = match &best {
                    None => true,
                    Some((s, l, _)) => *score > *s || (*score == *s && li < *l),
                };
                if better {
                    best = Some((*score, li, seg.clone()));
                }
            }
        }
    }
    best.map(|(_, _, s)| s)
}

/// Publication year: "(2021)", "© 2021", "Published … 2021" first, then the
/// file's creation date, then the most common year on page 1.
pub fn year_from(first_page: &str, created: Option<i32>, this_year: i32) -> Option<i32> {
    let chars: Vec<char> = first_page.chars().collect();
    let mut found: Vec<(i32, usize)> = Vec::new();
    let mut i = 0;
    while i + 4 <= chars.len() {
        let w: String = chars[i..i + 4].iter().collect();
        let before_digit = i > 0 && chars[i - 1].is_ascii_digit();
        let after_digit = i + 4 < chars.len() && chars[i + 4].is_ascii_digit();
        if !before_digit && !after_digit {
            if let Ok(y) = w.parse::<i32>() {
                if (1900..=this_year + 1).contains(&y) {
                    let start = i.saturating_sub(30);
                    let ctx: String = chars[start..i].iter().collect::<String>().to_lowercase();
                    let prev = if i > 0 { chars[i - 1] } else { ' ' };
                    let next = chars.get(i + 4).copied().unwrap_or(' ');
                    let strong = (prev == '(' && next == ')')
                        || ctx.contains('©')
                        || ctx.contains("copyright")
                        || ctx.contains("published")
                        || ctx.contains("accepted")
                        || ctx.contains("발행")
                        || ctx.contains("게재");
                    found.push((y, if strong { 3 } else { 1 }));
                }
            }
        }
        i += 1;
    }
    if let Some((y, _)) = found.iter().filter(|(_, w)| *w >= 3).max_by_key(|(y, _)| *y) {
        return Some(*y);
    }
    if let Some(y) = created.filter(|y| (1900..=this_year + 1).contains(y)) {
        return Some(y);
    }
    let mut counts: Vec<(i32, usize)> = Vec::new();
    for (y, _) in found {
        match counts.iter_mut().find(|(cy, _)| *cy == y) {
            Some(c) => c.1 += 1,
            None => counts.push((y, 1)),
        }
    }
    counts.into_iter().max_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0))).map(|(y, _)| y)
}

pub const DEFAULT_TEMPLATE: &str = "{저자}, {기관} ({연도})";

/// Fill a note-name pattern. Tokens: {저자} (short: "Doe et al."),
/// {저자전체}, {기관}, {연도}, {제목}, {파일}. Separators around empty
/// tokens are tidied away.
pub fn render(template: &str, authors: &str, institution: &str, year: &str, title: &str, file: &str) -> String {
    let short = format_authors(&split_authors(authors));
    let full = split_authors(authors).join(", ");
    let mut s = template.to_string();
    for (k, v) in [("{저자}", short.as_str()), ("{저자전체}", full.as_str()), ("{기관}", institution.trim()), ("{연도}", year.trim()), ("{제목}", title.trim()), ("{파일}", file.trim())] {
        s = s.replace(k, &format!("\u{1}{v}\u{1}"));
    }
    // Remove brackets around nothing and separators next to nothing.
    for empty in ["(\u{1}\u{1})", "[\u{1}\u{1}]", "（\u{1}\u{1}）"] {
        s = s.replace(empty, "");
    }
    let parts: Vec<&str> = s.split("\u{1}\u{1}").collect();
    let mut s = parts.join("");
    s = s.replace('\u{1}', "");
    loop {
        let before = s.clone();
        for (a, b) in [(", ,", ","), (",,", ","), (", (", " ("), ("  ", " "), (" ,", ","), ("- -", "-"), ("· ·", "·"), ("_ _", "_")] {
            s = s.replace(a, b);
        }
        if s == before {
            break;
        }
    }
    s.trim().trim_matches(|c: char| c == ',' || c == '-' || c == '·' || c == '_' || c.is_whitespace()).trim().to_string()
}

/// "저자, 기관 (연도)", leaving out what isn't known.
#[allow(dead_code)]
pub fn note_name(authors: &str, institution: &str, year: &str) -> String {
    let n = render(DEFAULT_TEMPLATE, authors, institution, year, "", "");
    if !n.is_empty() {
        return n;
    }
    note_name_old(authors, institution, year)
}

#[allow(dead_code)]
fn note_name_old(authors: &str, institution: &str, year: &str) -> String {
    let mut s = String::new();
    let a = format_authors(&split_authors(authors));
    if !a.is_empty() {
        s.push_str(&a);
    }
    if !institution.trim().is_empty() {
        if !s.is_empty() {
            s.push_str(", ");
        }
        s.push_str(institution.trim());
    }
    if !year.trim().is_empty() {
        if !s.is_empty() {
            s.push(' ');
        }
        s.push_str(&format!("({})", year.trim()));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(note_name("Jane Doe, John Smith, Ann Lee", "Seoul National University", "2021"), "Doe et al., Seoul National University (2021)");
        assert_eq!(note_name("김철수, 이영희", "한국과학기술연구원", "2019"), "김철수·이영희, 한국과학기술연구원 (2019)");
        assert_eq!(note_name("Jane Doe", "", "2020"), "Doe (2020)");
        let page = "Deep Learning for Cells\nJane Doe1*, John Smith2 and Ann Lee1\n1 Department of Biology, Seoul National University, Seoul, Korea\n2 Max Planck Institute\nReceived 3 May 2020; published 1 June 2021\n© 2021 Elsevier";
        assert_eq!(authors_from_text(page).as_deref(), Some("Jane Doe1*, John Smith2 and Ann Lee1"));
        assert_eq!(institution_from_text(page).as_deref(), Some("Seoul National University"));
        assert_eq!(year_from(page, Some(2023), 2026), Some(2021));
        assert_eq!(render("{저자}, {기관} ({연도})", "", "", "2020", "", ""), "(2020)");
        assert_eq!(render("{저자}, {기관} ({연도})", "Jane Doe", "", "", "", ""), "Doe");
        assert_eq!(render("{연도} - {저자} - {제목}", "Jane Doe", "", "2020", "Cells", ""), "2020 - Doe - Cells");
        assert_eq!(render("{저자} ({연도}) {제목}", "", "", "", "Cells", ""), "Cells");
    }
}
