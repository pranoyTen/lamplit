// The library's rules, kept apart from the UI so they can be tested: which books show, in what order, and how they are labelled.
use crate::api::Book;
use std::collections::{HashMap, HashSet};

/// "12h 4m" or "38m", like the Tauri app.
pub fn length(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    let (h, m) = (s / 3600, s % 3600 / 60);
    if h > 0 { format!("{h}h {m}m") } else { format!("{m}m") }
}

/// The colour a cover shows before its image loads: a muted hue picked from the title, the same one the Tauri app picks.
pub fn tint(title: &str) -> (u8, u8, u8) {
    let hue = title.encode_utf16().fold(7u32, |a, c| (a * 31 + c as u32) % 360) as f64;
    hsl(hue, 0.30, 0.28)
}

fn hsl(h: f64, s: f64, l: f64) -> (u8, u8, u8) {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let (r, g, b) = match (h / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    let to = |v: f64| ((v + m) * 255.0).round() as u8;
    (to(r), to(g), to(b))
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Filter {
    All,
    InProgress,
    NotStarted,
    Finished,
    Downloaded,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Sort {
    Title,
    Author,
    Added,
    Longest,
}

/// The books the Library shows: those matching the filter and the search text, in the chosen order.
pub fn visible<'a>(books: &'a [Book], progress: &HashMap<String, f64>, downloaded: &HashSet<String>, query: &str, filter: Filter, sort: Sort) -> Vec<&'a Book> {
    let q = query.trim().to_lowercase();
    let p = |b: &Book| progress.get(&b.id).copied().unwrap_or(0.0);
    let mut out: Vec<&Book> = books.iter()
        .filter(|b| match filter {
            Filter::All => true,
            Filter::InProgress => p(b) > 0.0 && p(b) < 1.0,
            Filter::NotStarted => p(b) == 0.0,
            Filter::Finished => p(b) >= 1.0,
            Filter::Downloaded => downloaded.contains(&b.id),
        })
        .filter(|b| q.is_empty() || format!("{} {}", b.title, b.author).to_lowercase().contains(&q))
        .collect();
    let key = |s: &str| s.to_lowercase();
    match sort {
        Sort::Title => out.sort_by_key(|b| key(&b.title)),
        Sort::Author => out.sort_by(|a, b| key(&a.author).cmp(&key(&b.author)).then_with(|| key(&a.title).cmp(&key(&b.title)))),
        Sort::Added => out.sort_by(|a, b| b.added.total_cmp(&a.added)),
        Sort::Longest => out.sort_by(|a, b| b.duration.total_cmp(&a.duration)),
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(id: &str, title: &str, author: &str, duration: f64, added: f64) -> Book {
        Book { id: id.into(), title: title.into(), author: author.into(), duration, added, library_id: String::new() }
    }

    #[test]
    fn lengths() {
        assert_eq!(length(3600.0 * 12.0 + 240.0), "12h 4m");
        assert_eq!(length(38.0 * 60.0), "38m");
    }

    #[test]
    fn tints_match_the_tauri_app() {
        // the Tauri app gives "Dune" hsl(223 30% 28%)
        assert_eq!(tint("Dune"), hsl(223.0, 0.30, 0.28));
        assert_eq!(hsl(0.0, 0.0, 0.5), (128, 128, 128));
    }

    #[test]
    fn filters_searches_and_sorts() {
        let books = [b("1", "Dune", "Herbert", 75000.0, 3.0), b("2", "Anathem", "Stephenson", 120000.0, 1.0), b("3", "Emma", "Austen", 50000.0, 2.0)];
        let progress = HashMap::from([("1".to_string(), 0.4), ("3".to_string(), 1.0)]);
        let dl = HashSet::from(["2".to_string()]);
        let ids = |v: Vec<&Book>| v.iter().map(|b| b.id.as_str()).collect::<Vec<_>>().join(",");
        assert_eq!(ids(visible(&books, &progress, &dl, "", Filter::All, Sort::Title)), "2,1,3");
        assert_eq!(ids(visible(&books, &progress, &dl, "", Filter::All, Sort::Author)), "3,1,2");
        assert_eq!(ids(visible(&books, &progress, &dl, "", Filter::All, Sort::Added)), "1,3,2");
        assert_eq!(ids(visible(&books, &progress, &dl, "", Filter::All, Sort::Longest)), "2,1,3");
        assert_eq!(ids(visible(&books, &progress, &dl, "", Filter::InProgress, Sort::Title)), "1");
        assert_eq!(ids(visible(&books, &progress, &dl, "", Filter::NotStarted, Sort::Title)), "2");
        assert_eq!(ids(visible(&books, &progress, &dl, "", Filter::Finished, Sort::Title)), "3");
        assert_eq!(ids(visible(&books, &progress, &dl, "  HERB ", Filter::All, Sort::Title)), "1");
        assert_eq!(ids(visible(&books, &progress, &dl, "", Filter::Downloaded, Sort::Title)), "2");
    }
}
