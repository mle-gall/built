// SPDX-License-Identifier: GPL-3.0-or-later
/// Plain text matches anywhere. A wildcard pattern matches the whole field,
/// and a question mark consumes one Unicode character.
pub(crate) fn text_matches(text: &str, token: &str) -> bool {
    if !token.contains(['*', '?']) {
        return text.contains(token);
    }
    let mut pattern = token.chars();
    let mut text = text.chars();
    let mut star = None;
    while let Some(ch) = text.clone().next() {
        match pattern.clone().next() {
            Some('*') => {
                pattern.next();
                star = Some((pattern.clone(), text.clone()));
            }
            Some(p) if p == '?' || p == ch => {
                pattern.next();
                text.next();
            }
            _ => {
                let Some((after_star, matched)) = &mut star else {
                    return false;
                };
                if matched.next().is_none() {
                    return false;
                }
                pattern = after_star.clone();
                text = matched.clone();
            }
        }
    }
    pattern.all(|ch| ch == '*')
}
