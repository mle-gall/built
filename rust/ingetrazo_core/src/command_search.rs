// SPDX-License-Identifier: GPL-3.0-or-later
//! Blender's F3 word scoring (string_search.cc, © Blender Authors), shared
//! by the command palette and the remaining composer host. The adapter folds
//! localized strings with Unicode NFKD and case folding before indexing.
use pyo3::prelude::*;
use pyo3::types::PyTuple;

#[pyclass(frozen)]
pub struct SearchItem {
    words: Vec<Vec<char>>,
    main: usize,
    name_len: usize,
    total_len: usize,
}

fn words(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| c.is_whitespace() || matches!(c, '-' | '_' | '/' | '▸'))
        .filter(|s| !s.is_empty())
}

// unicodedata and str.casefold share Python's Unicode tables with translated
// plugin labels. Keep a bounded cache on the module's callable.
#[pyfunction]
pub fn command_fold(py: Python<'_>, text: &str) -> PyResult<String> {
    let unicode = py.import("unicodedata")?;
    let normalized: String = unicode
        .call_method1("normalize", ("NFKD", text))?
        .extract()?;
    let combining = unicode.getattr("combining")?;
    let mut bare = String::with_capacity(normalized.len());
    for c in normalized.chars() {
        if c.is_ascii() || !combining.call1((c,))?.is_truthy()? {
            bare.push(c);
        }
    }
    pyo3::types::PyString::new(py, &bare)
        .call_method0("casefold")?
        .extract()
}

#[pymethods]
impl SearchItem {
    #[new]
    fn new(py: Python<'_>, groups: Vec<String>) -> PyResult<Self> {
        let fold = py.import("ingetrazo_core")?.getattr("command_fold")?;
        let mut terms = Vec::new();
        let mut main = 0;
        let mut total_len = 0;
        for group in groups.into_iter().filter(|s| !s.trim().is_empty()) {
            let folded: String = fold.call1((&group,))?.extract()?;
            let size = group.chars().count();
            main = terms.len();
            terms.extend(words(&folded).map(|w| w.chars().collect::<Vec<_>>()));
            total_len += size;
        }
        let name_len = terms[main..].iter().map(Vec::len).sum();
        Ok(Self {
            words: terms,
            main,
            name_len,
            total_len,
        })
    }

    #[getter]
    fn words<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        PyTuple::new(py, self.words.iter().map(|w| w.iter().collect::<String>()))
    }

    #[getter]
    fn main<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        PyTuple::new(py, (0..self.words.len()).map(|i| i >= self.main))
    }

    #[getter]
    fn main_length(&self) -> usize {
        self.name_len
    }

    #[getter]
    fn total_length(&self) -> usize {
        self.total_len
    }
}

#[pyfunction]
pub fn command_score(terms: Vec<String>, item: &SearchItem) -> Option<i64> {
    item.score(
        &terms
            .iter()
            .map(|s| s.chars().collect())
            .collect::<Vec<_>>(),
    )
}

#[pyfunction]
pub fn fuzzy_errors(query: &str, full: &str) -> i64 {
    errors(
        &query.chars().collect::<Vec<_>>(),
        &full.chars().collect::<Vec<_>>(),
    )
    .map_or(-1, |n| n as i64)
}

fn distance(a: &[char], b: &[char]) -> usize {
    let mut prev: Vec<_> = (0..=b.len()).collect();
    let mut older = vec![0; b.len() + 1];
    let mut next = older.clone();
    for (i, ca) in a.iter().enumerate() {
        next[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let mut cost = (prev[j + 1] + 1)
                .min(next[j] + 1)
                .min(prev[j] + usize::from(ca != cb));
            if i > 0 && j > 0 && *ca == b[j - 1] && a[i - 1] == *cb {
                cost = cost.min(older[j - 1] + 1);
            }
            next[j + 1] = cost;
        }
        std::mem::swap(&mut older, &mut prev);
        std::mem::swap(&mut prev, &mut next);
    }
    prev[b.len()]
}

#[pyfunction]
pub fn damerau_levenshtein(a: &str, b: &str) -> usize {
    distance(
        &a.chars().collect::<Vec<_>>(),
        &b.chars().collect::<Vec<_>>(),
    )
}

fn errors(query: &[char], full: &[char]) -> Option<usize> {
    if query.is_empty() {
        return Some(0);
    }
    if full.windows(query.len()).any(|part| part == query) {
        return Some(0);
    }
    let max = query.len() / 8 + 1;
    if query.len() < 2 || query.len() > full.len() + max {
        return None;
    }
    let window = (query.len() + max).min(full.len());
    let mut start = 0;
    while start + window <= full.len() {
        let mut skip = 1;
        if query[..2].contains(&full[start]) {
            let d = distance(query, &full[start..start + window]);
            if d <= max + window.saturating_sub(query.len()) {
                return Some(d);
            }
            skip = (d / 2).max(1);
        }
        if start + window == full.len() {
            break;
        }
        start = (start + skip).min(full.len() - window);
    }
    None
}

impl SearchItem {
    fn initials(&self, query: &[char], used: &[Option<usize>]) -> Option<Vec<usize>> {
        let mut best = None;
        let mut best_main = 0;
        for start in 0..self.words.len() {
            if used[start].is_some() || self.words[start][0] != query[0] {
                continue;
            }
            let (mut row, mut at) = (start, 0);
            let mut matched = Vec::new();
            for c in query {
                while row < self.words.len()
                    && (used[row].is_some() || self.words[row].get(at) != Some(c))
                {
                    row += 1;
                    at = 0;
                }
                if row == self.words.len() {
                    break;
                }
                matched.push(row);
                at += 1;
            }
            if matched.len() == query.len() {
                let main = matched.iter().filter(|&&i| i >= self.main).count();
                if best.is_none() || main > best_main {
                    best = Some(matched);
                    best_main = main;
                }
                if main == query.len() {
                    break;
                }
            }
        }
        best
    }

    fn score(&self, terms: &[Vec<char>]) -> Option<i64> {
        let mut used = vec![None; self.words.len()];
        let mut score = 1000;
        for (qi, q) in terms.iter().enumerate() {
            let shortest = terms[qi + 1..].iter().any(|term| term.starts_with(q));
            let mut best: Option<usize> = None;
            for (i, w) in self.words.iter().enumerate() {
                if used[i].is_some() || !w.starts_with(q) {
                    continue;
                }
                if best.is_none_or(|j| {
                    if shortest {
                        w.len() < self.words[j].len()
                    } else {
                        j < self.main
                    }
                }) {
                    best = Some(i);
                }
            }
            if let Some(i) = best {
                used[i] = Some(qi);
                score += if i >= self.main { 10 } else { 9 };
                continue;
            }
            if q.is_empty() {
                return None;
            }
            if let Some(indices) = self.initials(q, &used) {
                score += if indices.iter().all(|&i| i >= self.main) {
                    4
                } else {
                    3
                };
                for i in indices {
                    used[i] = Some(qi);
                }
                continue;
            }
            let (i, err) = self
                .words
                .iter()
                .enumerate()
                .filter(|(i, _)| used[*i].is_none())
                .find_map(|(i, w)| errors(q, w).map(|err| (i, err)))?;
            used[i] = Some(qi);
            score += 3 - err as i64;
        }
        let order: Vec<_> = used.into_iter().flatten().collect();
        Some(score - order.windows(2).filter(|pair| pair[0] > pair[1]).count() as i64)
    }
}

#[pyfunction]
pub fn rank_commands(
    folded: &str,
    short_query: bool,
    entries: Vec<(Vec<Py<SearchItem>>, i64)>,
) -> Vec<usize> {
    let terms: Vec<Vec<char>> = words(folded).map(|w| w.chars().collect()).collect();
    let mut matches = Vec::new();
    for (order, (aliases, recent)) in entries.iter().enumerate() {
        let mut best = None;
        for item in aliases {
            let item = item.get();
            if let Some(score) = item.score(&terms) {
                if best.is_none_or(|(s, _, _)| score > s) {
                    best = Some((score, item.name_len, item.total_len));
                }
            }
        }
        if let Some((score, size, total)) = best {
            matches.push((score, size, total, *recent, order));
        }
    }
    let Some(top) = matches.iter().map(|r| r.0).max() else {
        return vec![];
    };
    matches.sort_by_key(|&(score, size, total, recent, index)| {
        let best = score == top;
        (
            std::cmp::Reverse(score),
            std::cmp::Reverse(if best && short_query { recent } else { 0 }),
            if best && !terms.is_empty() {
                (size, total)
            } else {
                (0, 0)
            },
            index,
        )
    });
    matches.into_iter().map(|r| r.4).collect()
}
