//! Deterministic fuzzy matching shared by command and resource pickers.
pub fn score(query: &str, candidate: &str) -> Option<usize> {
    let q: Vec<char> = query.to_lowercase().chars().collect();
    let c: Vec<char> = candidate.to_lowercase().chars().collect();
    if q.is_empty() {
        return Some(0);
    }
    if q.len() > c.len() {
        return None;
    }
    let mut pos = 0;
    let mut first = None;
    let mut gaps = 0;
    for ch in q {
        let found = c[pos..].iter().position(|x| *x == ch)?;
        let absolute = pos + found;
        if first.is_none() {
            first = Some(absolute)
        }
        gaps += found;
        pos = absolute + 1
    }
    Some(first.unwrap_or(0) * 10 + gaps)
}
pub fn rank_strings<'a>(query: &str, values: impl IntoIterator<Item = &'a str>) -> Vec<&'a str> {
    let mut ranked: Vec<(usize, usize, &'a str)> = values
        .into_iter()
        .enumerate()
        .filter_map(|(i, v)| score(query, v).map(|s| (s, i, v)))
        .collect();
    ranked.sort_by_key(|(s, i, _)| (*s, *i));
    ranked.into_iter().map(|(_, _, v)| v).collect()
}
pub fn rank_by<T>(
    query: &str,
    values: impl IntoIterator<Item = T>,
    label: impl Fn(&T) -> &str,
) -> Vec<T> {
    let mut ranked: Vec<(usize, usize, T)> = values
        .into_iter()
        .enumerate()
        .filter_map(|(i, v)| score(query, label(&v)).map(|s| (s, i, v)))
        .collect();
    ranked.sort_by_key(|(s, i, _)| (*s, *i));
    ranked.into_iter().map(|(_, _, v)| v).collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unicode() {
        assert!(score("项β", "项目-β").is_some());
        assert!(score("xyz", "example").is_none());
    }
    #[test]
    fn ranking() {
        let got = rank_strings("res", ["restart", "resume", "reset"]);
        assert_eq!(got, vec!["restart", "resume", "reset"]);
    }
    #[test]
    fn empty() {
        assert_eq!(rank_strings("", ["b", "a"]), vec!["b", "a"]);
    }
}
