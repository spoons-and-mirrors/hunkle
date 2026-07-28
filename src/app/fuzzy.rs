pub(super) fn fuzzy_text_score(query: &str, candidate: &str) -> Option<u32> {
    let query = query.to_lowercase();
    let candidate = candidate.to_lowercase();
    fuzzy_text_score_lower(&query, &candidate)
}

pub(super) fn fuzzy_text_score_lower(query: &str, candidate: &str) -> Option<u32> {
    let query_len = query.chars().count();
    if query == candidate {
        return Some(10_000);
    }
    if candidate.starts_with(query) {
        return Some(9_000u32.saturating_sub(candidate.chars().count() as u32));
    }
    if let Some(index) = candidate.find(query) {
        return Some(8_000u32.saturating_sub(candidate[..index].chars().count() as u32));
    }
    let mut first = None;
    let mut last = 0;
    let mut characters = candidate.chars().enumerate();
    for needle in query.chars() {
        let (index, _) = characters.find(|(_, candidate)| *candidate == needle)?;
        first.get_or_insert(index);
        last = index;
    }
    let span = last - first?;
    if span > query_len.saturating_mul(3).max(4) {
        return None;
    }
    Some(6_000u32.saturating_sub(span as u32))
}

#[cfg(test)]
mod tests {
    use super::fuzzy_text_score;

    #[test]
    fn ranks_exact_prefix_and_subsequence_matches() {
        let exact = fuzzy_text_score("app", "app").unwrap();
        let prefix = fuzzy_text_score("app", "application").unwrap();
        let subsequence = fuzzy_text_score("app", "src/a-p-p.rs").unwrap();

        assert!(exact > prefix);
        assert!(prefix > subsequence);
        assert!(fuzzy_text_score("app", "unrelated").is_none());
    }

    #[test]
    fn scores_unicode_by_characters_instead_of_bytes() {
        assert!(fuzzy_text_score("éx", "é---x").is_some());
        assert_eq!(fuzzy_text_score("x", "éx"), fuzzy_text_score("x", "ax"));
    }
}
