// SPDX-License-Identifier: GPL-3.0-or-later
//! Runs of equal elements between two versions of an array.
//!
//! An edit of a big mesh rewrites a few faces and edges in place and
//! appends the new ones, so the arrays baked from it before and after are
//! mostly the same elements, shifted. `runs` finds those stretches by
//! comparing elements (never by trusting an outside diff): whatever it
//! pairs is equal, so a consumer reusing what it derived from an old
//! element for the new one it is paired with is right by construction.
//! A poor alignment only costs reuse.

/// `(old_start, new_start, len)`: `old[o..o + len] == new[n..n + len]`,
/// both ascending and disjoint, or None when the two differ past what a
/// local edit explains (more than `window` elements to skip at some point
/// before they agree again on `confirm` elements in a row).
pub fn runs<T: PartialEq>(old: &[T], new: &[T], window: usize, confirm: usize) -> Option<Vec<(usize, usize, usize)>> {
    runs_by(old.len(), new.len(), |i, j| old[i] == new[j], window, confirm)
}

/// `runs` over elements told equal by `same(old index, new index)`.
pub fn runs_by(n_old: usize, n_new: usize, same: impl Fn(usize, usize) -> bool, window: usize, confirm: usize)
    -> Option<Vec<(usize, usize, usize)>> {
    let mut out = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);
    let agree = |i: usize, j: usize| -> bool {
        let k = confirm.min(n_old - i).min(n_new - j).max(1);
        (0..k).all(|t| same(i + t, j + t))
    };
    loop {
        let (si, sj) = (i, j);
        while i < n_old && j < n_new && same(i, j) {
            i += 1;
            j += 1;
        }
        if i > si {
            out.push((si, sj, i - si));
        }
        if i == n_old || j == n_new {
            return Some(out);
        }
        // Skip the fewest elements (old and new together) after which the
        // two agree again.
        let mut found = None;
        'search: for d in 1..=window {
            for di in 0..=d {
                let dj = d - di;
                if i + di < n_old && j + dj < n_new && agree(i + di, j + dj) {
                    found = Some((di, dj));
                    break 'search;
                }
            }
            if i + d >= n_old && j + d >= n_new {
                break;
            }
        }
        match found {
            Some((di, dj)) => {
                i += di;
                j += dj;
            }
            // Whatever follows one end is new or gone on the other.
            None if i + window >= n_old || j + window >= n_new => return Some(out),
            None => return None,
        }
    }
}

/// How many elements of `new` the runs leave unpaired.
pub fn unpaired(runs: &[(usize, usize, usize)], n_new: usize) -> usize {
    n_new - runs.iter().map(|r| r.2).sum::<usize>()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(old: &[u32], new: &[u32], runs: &[(usize, usize, usize)]) {
        let (mut lo, mut ln) = (0, 0);
        for &(o, n, len) in runs {
            assert!(o >= lo && n >= ln && len > 0);
            assert_eq!(old[o..o + len], new[n..n + len]);
            lo = o + len;
            ln = n + len;
        }
    }

    #[test]
    fn pairs_what_an_edit_kept() {
        let old: Vec<u32> = (0..1000).collect();
        // Faces 100..102 rewritten (one more element), 500 removed, two appended.
        let mut new: Vec<u32> = (0..100).collect();
        new.extend([5000, 5001, 5002]);
        new.extend(102..500);
        new.extend(501..1000);
        new.extend([7000, 7001]);
        let r = runs(&old, &new, 64, 4).unwrap();
        check(&old, &new, &r);
        assert_eq!(unpaired(&r, new.len()), 5);
    }

    #[test]
    fn gives_up_on_a_wholesale_change() {
        let old: Vec<u32> = (0..1000).collect();
        let new: Vec<u32> = (2000..3000).collect();
        assert!(runs(&old, &new, 64, 4).is_none());
        // Short arrays: what is past the window counts as new.
        let r = runs(&old[..10], &new[..10], 64, 4).unwrap();
        assert!(r.is_empty());
    }

    #[test]
    fn appended_and_truncated_tails() {
        let old: Vec<u32> = (0..50).collect();
        let longer: Vec<u32> = (0..60).collect();
        assert_eq!(runs(&old, &longer, 8, 4).unwrap(), vec![(0, 0, 50)]);
        assert_eq!(runs(&longer, &old, 8, 4).unwrap(), vec![(0, 0, 50)]);
    }
}
