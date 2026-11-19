// Turns the server's listening stats into the numbers and bars the Stats page shows. A port of the Tauri app's stats.js.
use chrono::{Datelike, Duration, NaiveDate};
use std::collections::HashMap;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Range {
    Week,  // the last 7 days
    Month, // this calendar month so far
    Year,  // this calendar year so far
}

#[derive(Debug, PartialEq)]
pub struct Summary {
    pub time: f64,
    pub avg: f64, // per day in the range
    pub streak: u32,
    pub bars: Vec<(String, f64)>,
    pub authors: Vec<(String, f64)>, // all time, top five
    pub finished: usize,             // books finished in the range
}

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// `days`: seconds listened per local day ("YYYY-MM-DD"); `authors`: (author, seconds) per book; `finished`: local days books were finished.
pub fn summarize(days: &HashMap<String, f64>, authors: &[(String, f64)], finished: &[NaiveDate], range: Range, today: NaiveDate) -> Summary {
    let sec = |d: NaiveDate| days.get(&d.format("%Y-%m-%d").to_string()).copied().unwrap_or(0.0);
    let (bars, from): (Vec<(String, f64)>, NaiveDate) = match range {
        Range::Week => {
            let from = today - Duration::days(6);
            ((0..7).map(|i| from + Duration::days(i)).map(|d| (d.format("%a").to_string(), sec(d))).collect(), from)
        }
        Range::Month => {
            let from = today.with_day(1).unwrap();
            ((1..=today.day()).map(|n| (n.to_string(), sec(today.with_day(n).unwrap()))).collect(), from)
        }
        Range::Year => {
            let from = NaiveDate::from_ymd_opt(today.year(), 1, 1).unwrap();
            let bars = (1..=today.month())
                .map(|m| {
                    let prefix = format!("{}-{m:02}-", today.year());
                    (MONTHS[m as usize - 1].to_string(), days.iter().filter(|(k, _)| k.starts_with(&prefix)).map(|(_, s)| s).sum())
                })
                .collect();
            (bars, from)
        }
    };
    let span = (today - from).num_days() + 1;
    let time: f64 = bars.iter().map(|b| b.1).sum();
    // days in a row with listening, counting from today, or from yesterday when nothing yet today
    let mut day = if sec(today) > 0.0 { today } else { today - Duration::days(1) };
    let mut streak = 0;
    while sec(day) > 0.0 {
        streak += 1;
        day -= Duration::days(1);
    }
    let mut by_author: HashMap<&str, f64> = HashMap::new();
    for (a, s) in authors.iter().filter(|(a, _)| !a.is_empty()) {
        *by_author.entry(a).or_default() += s;
    }
    let mut top: Vec<(String, f64)> = by_author.into_iter().map(|(a, s)| (a.to_string(), s)).collect();
    top.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    top.truncate(5);
    Summary { time, avg: time / span as f64, streak, bars, authors: top, finished: finished.iter().filter(|d| **d >= from && **d <= today).count() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sums_ranges_streaks_and_authors() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 19).unwrap();
        let days = HashMap::from([
            ("2026-11-19".to_string(), 600.0),
            ("2026-11-18".to_string(), 1200.0),
            ("2026-11-17".to_string(), 60.0),
            ("2026-11-15".to_string(), 300.0),
            ("2026-10-02".to_string(), 3600.0),
            ("2025-12-31".to_string(), 999.0),
        ]);
        let authors = vec![("Herbert".to_string(), 100.0), ("Austen".to_string(), 500.0), ("Herbert".to_string(), 450.0), (String::new(), 9.0)];
        let finished = [NaiveDate::from_ymd_opt(2026, 11, 3).unwrap(), NaiveDate::from_ymd_opt(2026, 3, 1).unwrap()];

        let w = summarize(&days, &authors, &finished, Range::Week, today);
        assert_eq!(w.bars.len(), 7);
        assert_eq!(w.bars[6], ("Thu".to_string(), 600.0)); // 19 Nov 2026 is a Thursday
        assert_eq!(w.time, 2160.0);
        assert_eq!(w.avg, 2160.0 / 7.0);
        assert_eq!(w.streak, 3);
        assert_eq!(w.finished, 0);
        assert_eq!(w.authors, vec![("Herbert".to_string(), 550.0), ("Austen".to_string(), 500.0)]);

        let m = summarize(&days, &authors, &finished, Range::Month, today);
        assert_eq!(m.bars.len(), 19);
        assert_eq!(m.finished, 1);

        let y = summarize(&days, &authors, &finished, Range::Year, today);
        assert_eq!(y.bars.len(), 11);
        assert_eq!(y.bars[9], ("Oct".to_string(), 3600.0));
        assert_eq!(y.time, 5760.0); // last year's day is left out
        assert_eq!(y.finished, 2);

        // nothing yet today: the streak counts back from yesterday
        let mut quiet = days.clone();
        quiet.remove("2026-11-19");
        assert_eq!(summarize(&quiet, &[], &[], Range::Week, today).streak, 2);
    }
}
