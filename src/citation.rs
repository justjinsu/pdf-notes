//! "How do I cite this?" — reference entries in common styles, an in-text
//! form and a BibTeX entry, built from the document's details. With a DOI the
//! details can be completed from CrossRef (the original looks DOIs up there too).

use serde::{Deserialize, Serialize};

use crate::model::DocNotes;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Person {
    pub family: String,
    pub given: String,
}

fn is_hangul(s: &str) -> bool {
    s.chars().any(|c| ('\u{AC00}'..='\u{D7A3}').contains(&c))
}

impl Person {
    /// "Jane Q. Doe", "Doe, Jane", "김철수".
    pub fn parse(name: &str) -> Person {
        let name = name.trim();
        if let Some((f, g)) = name.split_once(',') {
            return Person { family: f.trim().into(), given: g.trim().into() };
        }
        if is_hangul(name) {
            return Person { family: name.split_whitespace().collect::<Vec<_>>().join(""), given: String::new() };
        }
        let mut words: Vec<&str> = name.split_whitespace().collect();
        let family = words.pop().unwrap_or_default().to_string();
        Person { family, given: words.join(" ") }
    }
    fn korean(&self) -> bool {
        is_hangul(&self.family)
    }
    /// "J. Q."
    fn initials(&self) -> String {
        self.given
            .split(|c: char| c.is_whitespace() || c == '.')
            .filter(|w| !w.is_empty())
            .map(|w| {
                // "Jean-Paul" -> "J.-P."
                w.split('-').map(|p| format!("{}.", p.chars().next().unwrap_or(' '))).collect::<Vec<_>>().join("-")
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
    /// "Doe, J. Q." (APA) / "김철수".
    fn apa(&self) -> String {
        if self.korean() || self.given.is_empty() {
            format!("{}{}", self.family, self.given)
        } else {
            format!("{}, {}", self.family, self.initials())
        }
    }
    fn full(&self) -> String {
        if self.korean() || self.given.is_empty() {
            format!("{}{}", self.family, self.given)
        } else {
            format!("{} {}", self.given, self.family)
        }
    }
    fn inverted(&self) -> String {
        if self.korean() || self.given.is_empty() {
            self.full()
        } else {
            format!("{}, {}", self.family, self.given)
        }
    }
}

/// Everything a reference entry needs.
#[derive(Clone, Debug, Default)]
pub struct Work {
    pub title: String,
    pub authors: Vec<Person>,
    pub year: String,
    pub journal: String,
    pub volume: String,
    pub issue: String,
    pub pages: String,
    pub publisher: String,
    pub doi: String,
}

impl Work {
    pub fn from_notes(n: &DocNotes) -> Work {
        let authors = if !n.authors.is_empty() {
            n.authors.clone()
        } else {
            crate::naming::split_authors(&n.author).iter().map(|a| Person::parse(a)).collect()
        };
        let title = if n.title.trim().is_empty() {
            std::path::Path::new(&n.pdf_path).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()
        } else {
            n.title.trim().to_string()
        };
        Work {
            title,
            authors,
            year: n.year.trim().to_string(),
            journal: n.journal.trim().to_string(),
            volume: n.volume.trim().to_string(),
            issue: n.issue.trim().to_string(),
            pages: n.pages.trim().replace("--", "–").replace('-', "–"),
            publisher: n.publisher.trim().to_string(),
            doi: n.doi.trim().to_string(),
        }
    }

    fn year_or_nd(&self) -> String {
        if self.year.is_empty() { "n.d.".into() } else { self.year.clone() }
    }

    fn doi_url(&self) -> Option<String> {
        if self.doi.is_empty() { None } else { Some(format!("https://doi.org/{}", self.doi)) }
    }

    fn korean(&self) -> bool {
        self.authors.first().map(|a| a.korean()).unwrap_or(false)
    }

    /// (Doe, 2021) · (Doe & Smith, 2021) · (Doe et al., 2021) · (김철수 외, 2021)
    pub fn in_text(&self) -> String {
        let y = self.year_or_nd();
        let a = &self.authors;
        let who = match a.len() {
            0 => short_title(&self.title),
            1 => a[0].family.clone(),
            2 if self.korean() => format!("{}, {}", a[0].family, a[1].family),
            2 => format!("{} & {}", a[0].family, a[1].family),
            _ if self.korean() => format!("{} 외", a[0].family),
            _ => format!("{} et al.", a[0].family),
        };
        format!("({who}, {y})")
    }

    /// Narrative form: "Doe et al. (2021)".
    pub fn narrative(&self) -> String {
        let t = self.in_text();
        let inner = &t[1..t.len() - 1];
        match inner.rsplit_once(", ") {
            Some((who, y)) => format!("{} ({})", who.replace(" & ", " and "), y),
            None => t,
        }
    }

    /// APA 7th edition reference entry (Markdown italics).
    pub fn apa(&self) -> String {
        let a = &self.authors;
        let names: Vec<String> = a.iter().map(|p| p.apa()).collect();
        let authors = match names.len() {
            0 => String::new(),
            1 => names[0].clone(),
            2..=20 => {
                if self.korean() {
                    names.join(", ")
                } else {
                    format!("{}, & {}", names[..names.len() - 1].join(", "), names[names.len() - 1])
                }
            }
            _ => format!("{}, . . . {}", names[..19].join(", "), names[names.len() - 1]),
        };
        let mut s = String::new();
        if authors.is_empty() {
            s.push_str(&format!("{}. ({}).", self.title, self.year_or_nd()));
        } else {
            s.push_str(&authors);
            if !authors.ends_with('.') {
                s.push('.');
            }
            s.push_str(&format!(" ({}). ", self.year_or_nd()));
            if self.journal.is_empty() {
                s.push_str(&format!("*{}*.", self.title.trim_end_matches('.')));
            } else {
                s.push_str(&format!("{}.", self.title.trim_end_matches('.')));
            }
        }
        if !self.journal.is_empty() {
            s.push_str(&format!(" *{}*", self.journal));
            if !self.volume.is_empty() {
                s.push_str(&format!(", *{}*", self.volume));
                if !self.issue.is_empty() {
                    s.push_str(&format!("({})", self.issue));
                }
            }
            if !self.pages.is_empty() {
                s.push_str(&format!(", {}", self.pages));
            }
            s.push('.');
        } else if !self.publisher.is_empty() {
            s.push_str(&format!(" {}.", self.publisher));
        }
        if let Some(u) = self.doi_url() {
            s.push_str(&format!(" {u}"));
        }
        s
    }

    /// MLA 9th edition.
    pub fn mla(&self) -> String {
        let a = &self.authors;
        let authors = match a.len() {
            0 => String::new(),
            1 => a[0].inverted(),
            2 => format!("{}, and {}", a[0].inverted(), a[1].full()),
            _ => format!("{}, et al", a[0].inverted()),
        };
        let mut s = String::new();
        if !authors.is_empty() {
            s.push_str(&authors);
            s.push_str(". ");
        }
        if self.journal.is_empty() {
            s.push_str(&format!("*{}*.", self.title.trim_end_matches('.')));
            if !self.publisher.is_empty() {
                s.push_str(&format!(" {},", self.publisher));
            }
            if !self.year.is_empty() {
                s.push_str(&format!(" {}.", self.year));
            }
        } else {
            s.push_str(&format!("“{}.” *{}*", self.title.trim_end_matches('.'), self.journal));
            if !self.volume.is_empty() {
                s.push_str(&format!(", vol. {}", self.volume));
            }
            if !self.issue.is_empty() {
                s.push_str(&format!(", no. {}", self.issue));
            }
            if !self.year.is_empty() {
                s.push_str(&format!(", {}", self.year));
            }
            if !self.pages.is_empty() {
                s.push_str(&format!(", pp. {}", self.pages));
            }
            s.push('.');
        }
        if let Some(u) = self.doi_url() {
            s.push_str(&format!(" {u}."));
        }
        s
    }

    /// Chicago author-date.
    pub fn chicago(&self) -> String {
        let a = &self.authors;
        let authors = match a.len() {
            0 => String::new(),
            1 => a[0].inverted(),
            n => {
                let mut v = vec![a[0].inverted()];
                for p in &a[1..n.min(10)] {
                    v.push(p.full());
                }
                if n > 10 {
                    format!("{}, et al", v[..7].join(", "))
                } else {
                    let last = v.pop().unwrap();
                    format!("{}, and {}", v.join(", "), last)
                }
            }
        };
        let mut s = String::new();
        if !authors.is_empty() {
            s.push_str(&authors);
            s.push_str(". ");
        }
        s.push_str(&format!("{}. ", self.year_or_nd()));
        if self.journal.is_empty() {
            s.push_str(&format!("*{}*.", self.title.trim_end_matches('.')));
            if !self.publisher.is_empty() {
                s.push_str(&format!(" {}.", self.publisher));
            }
        } else {
            s.push_str(&format!("“{}.” *{}*", self.title.trim_end_matches('.'), self.journal));
            if !self.volume.is_empty() {
                s.push_str(&format!(" {}", self.volume));
            }
            if !self.issue.is_empty() {
                s.push_str(&format!(" ({})", self.issue));
            }
            if !self.pages.is_empty() {
                s.push_str(&format!(": {}", self.pages));
            }
            s.push('.');
        }
        if let Some(u) = self.doi_url() {
            s.push_str(&format!(" {u}."));
        }
        s
    }

    /// "doe2021circulating"
    pub fn citekey(&self) -> String {
        let fam = self.authors.first().map(|a| a.family.clone()).unwrap_or_else(|| "anon".into());
        let fam: String = fam.chars().filter(|c| c.is_alphanumeric()).collect::<String>().to_lowercase();
        let stop = ["a", "an", "the", "on", "of", "in", "for", "and", "to", "with"];
        let word: String = self
            .title
            .split_whitespace()
            .map(|w| w.chars().filter(|c| c.is_alphanumeric()).collect::<String>().to_lowercase())
            .find(|w| !w.is_empty() && !stop.contains(&w.as_str()))
            .unwrap_or_default();
        format!("{}{}{}", fam, self.year, word)
    }

    pub fn bibtex(&self) -> String {
        let kind = if self.journal.is_empty() { "misc" } else { "article" };
        let mut fields: Vec<(String, String)> = Vec::new();
        if !self.authors.is_empty() {
            fields.push(("author".into(), self.authors.iter().map(|p| p.inverted()).collect::<Vec<_>>().join(" and ")));
        }
        fields.push(("title".into(), format!("{{{}}}", self.title)));
        for (k, v) in [
            ("journal", &self.journal),
            ("year", &self.year),
            ("volume", &self.volume),
            ("number", &self.issue),
            ("publisher", &self.publisher),
            ("doi", &self.doi),
        ] {
            if !v.is_empty() {
                fields.push((k.into(), v.clone()));
            }
        }
        if !self.pages.is_empty() {
            fields.push(("pages".into(), self.pages.replace('–', "--")));
        }
        let body = fields.iter().map(|(k, v)| format!("  {k} = {{{v}}}")).collect::<Vec<_>>().join(",\n");
        format!("@{kind}{{{},\n{}\n}}", self.citekey(), body)
    }
}

fn short_title(t: &str) -> String {
    let words: Vec<&str> = t.split_whitespace().take(4).collect();
    format!("*{}*", words.join(" "))
}

/// Which parts the "인용" section shows.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CitationOptions {
    pub enabled: bool,
    pub in_text: bool,
    pub apa: bool,
    pub mla: bool,
    pub chicago: bool,
    pub bibtex: bool,
    /// Look the DOI up on CrossRef when a PDF is opened.
    pub auto_fetch: bool,
}

impl Default for CitationOptions {
    fn default() -> Self {
        CitationOptions { enabled: true, in_text: true, apa: true, mla: false, chicago: false, bibtex: true, auto_fetch: true }
    }
}

/// The "## 인용" section put at the end of the notes.
pub fn section(n: &DocNotes, o: &CitationOptions) -> String {
    if !o.enabled {
        return String::new();
    }
    let w = Work::from_notes(n);
    let mut s = String::from("\n## 인용 (Citation)\n\n");
    if o.in_text {
        s.push_str(&format!("- **본문 인용**: {} · {}\n", w.in_text(), w.narrative()));
    }
    if o.apa {
        s.push_str(&format!("- **APA 7**: {}\n", w.apa()));
    }
    if o.mla {
        s.push_str(&format!("- **MLA 9**: {}\n", w.mla()));
    }
    if o.chicago {
        s.push_str(&format!("- **Chicago**: {}\n", w.chicago()));
    }
    if o.bibtex {
        s.push_str(&format!("\n```bibtex\n{}\n```\n", w.bibtex()));
    }
    s
}

/// Details found on CrossRef for a DOI.
#[derive(Clone, Debug, Default)]
pub struct Found {
    pub title: String,
    pub authors: Vec<Person>,
    pub year: String,
    pub journal: String,
    pub volume: String,
    pub issue: String,
    pub pages: String,
    pub publisher: String,
}

/// Ask CrossRef about a DOI (sends only the DOI).
pub fn fetch_crossref(doi: &str) -> Result<Found, String> {
    let doi = doi.trim().trim_start_matches("https://doi.org/").trim_start_matches("doi:");
    let url = format!("https://api.crossref.org/works/{}", crate::obsidian::encode_component(doi).replace("%2F", "/"));
    let body = ureq::get(&url)
        .set("User-Agent", "PDF-Notes/1.0 (desktop PDF reader)")
        .timeout(std::time::Duration::from_secs(12))
        .call()
        .map_err(|e| match e {
            ureq::Error::Status(404, _) => "CrossRef에 이 DOI가 없어요.".to_string(),
            other => format!("CrossRef에 연결하지 못했어요: {other}"),
        })?
        .into_string()
        .map_err(|e| e.to_string())?;
    let v: serde_json::Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    let m = &v["message"];
    let first = |key: &str| m[key].as_array().and_then(|a| a.first()).and_then(|x| x.as_str()).unwrap_or("").to_string();
    let text = |key: &str| m[key].as_str().unwrap_or("").to_string();
    let mut year = String::new();
    for key in ["published-print", "published-online", "issued", "published"] {
        if let Some(y) = m[key]["date-parts"][0][0].as_i64() {
            year = y.to_string();
            break;
        }
    }
    let authors = m["author"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|p| {
                    let family = p["family"].as_str().or(p["name"].as_str())?.to_string();
                    Some(Person { family, given: p["given"].as_str().unwrap_or("").to_string() })
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(Found {
        title: first("title"),
        authors,
        year,
        journal: first("container-title"),
        volume: text("volume"),
        issue: text("issue"),
        pages: text("page"),
        publisher: text("publisher"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Work {
        Work {
            title: "Circulating exosomes as biomarkers".into(),
            authors: vec![Person::parse("Jane Q. Doe"), Person::parse("John Smith"), Person::parse("Ann Lee")],
            year: "2021".into(),
            journal: "Journal of Cells".into(),
            volume: "12".into(),
            issue: "3".into(),
            pages: "45–67".into(),
            publisher: String::new(),
            doi: "10.1234/abc.5".into(),
        }
    }

    #[test]
    #[ignore]
    fn crossref_lookup() {
        let f = fetch_crossref("10.1038/nature14539").unwrap();
        println!("{f:?}");
        assert_eq!(f.journal, "Nature");
        assert_eq!(f.year, "2015");
        assert_eq!(f.authors.len(), 3);
    }

    #[test]
    fn styles() {
        let w = sample();
        assert_eq!(w.in_text(), "(Doe et al., 2021)");
        assert_eq!(w.narrative(), "Doe et al. (2021)");
        assert_eq!(
            w.apa(),
            "Doe, J. Q., Smith, J., & Lee, A. (2021). Circulating exosomes as biomarkers. *Journal of Cells*, *12*(3), 45–67. https://doi.org/10.1234/abc.5"
        );
        assert!(w.mla().starts_with("Doe, Jane Q., et al. “Circulating exosomes as biomarkers.” *Journal of Cells*, vol. 12, no. 3, 2021, pp. 45–67."));
        assert!(w.chicago().starts_with("Doe, Jane Q., John Smith, and Ann Lee. 2021. “Circulating exosomes as biomarkers.” *Journal of Cells* 12 (3): 45–67."));
        assert!(w.bibtex().starts_with("@article{doe2021circulating,\n  author = {Doe, Jane Q. and Smith, John and Lee, Ann}"));
        assert!(w.bibtex().contains("pages = {45--67}"));
        let k = Work { authors: vec![Person::parse("김철수"), Person::parse("이영희"), Person::parse("박민수")], ..sample() };
        assert_eq!(k.in_text(), "(김철수 외, 2021)");
        assert!(k.apa().starts_with("김철수, 이영희, 박민수. (2021)."));
    }
}
