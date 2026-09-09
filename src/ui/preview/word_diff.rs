use std::ops::Range;

use super::diff::DiffLineKind;

const MAX_TOKENS_PER_LINE: usize = 600;
const MAX_LINE_BYTE_LEN: usize = 4000;
const MIN_SIMILARITY: f32 = 0.30;
const MAX_BLOCK_DP_CELLS: usize = 2500;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Token<'a> {
    pub(super) text: &'a str,
    pub(super) range: Range<usize>,
    pub(super) is_whitespace: bool,
}

pub(super) fn tokenize(input: &str) -> Vec<Token<'_>> {
    if input.len() > MAX_LINE_BYTE_LEN {
        return Vec::new();
    }
    let mut tokens = Vec::new();
    let mut char_indices = input.char_indices().peekable();

    while let Some((start, c)) = char_indices.next() {
        if c.is_whitespace() {
            let mut end = start + c.len_utf8();
            while let Some(&(_, next_c)) = char_indices.peek() {
                if next_c.is_whitespace() {
                    end += next_c.len_utf8();
                    char_indices.next();
                } else {
                    break;
                }
            }
            tokens.push(Token {
                text: &input[start..end],
                range: start..end,
                is_whitespace: true,
            });
        } else if c.is_alphanumeric() || c == '_' {
            let mut end = start + c.len_utf8();
            while let Some(&(_, next_c)) = char_indices.peek() {
                if next_c.is_alphanumeric() || next_c == '_' {
                    end += next_c.len_utf8();
                    char_indices.next();
                } else {
                    break;
                }
            }
            tokens.push(Token {
                text: &input[start..end],
                range: start..end,
                is_whitespace: false,
            });
        } else {
            let end = start + c.len_utf8();
            tokens.push(Token {
                text: &input[start..end],
                range: start..end,
                is_whitespace: false,
            });
        }
        if tokens.len() >= MAX_TOKENS_PER_LINE {
            return Vec::new();
        }
    }
    tokens
}

/// Compute the Longest Common Subsequence of non-whitespace tokens between old and new lines.
/// Whitespace between unchanged tokens is treated as unchanged, while whitespace around
/// changed tokens forms part of the changed span (trimmed at span boundaries).
/// Returns (similarity score, common_in_old, common_in_new).
pub(super) fn token_lcs(
    old_tokens: &[Token<'_>],
    new_tokens: &[Token<'_>],
) -> (f32, Vec<bool>, Vec<bool>) {
    let n = old_tokens.len();
    let m = new_tokens.len();
    if n == 0 || m == 0 {
        return (0.0, vec![false; n], vec![false; m]);
    }

    let old_nw: Vec<usize> = old_tokens
        .iter()
        .enumerate()
        .filter_map(|(i, t)| (!t.is_whitespace).then_some(i))
        .collect();
    let new_nw: Vec<usize> = new_tokens
        .iter()
        .enumerate()
        .filter_map(|(i, t)| (!t.is_whitespace).then_some(i))
        .collect();

    let mut common_old = vec![false; n];
    let mut common_new = vec![false; m];

    if old_nw.is_empty() && new_nw.is_empty() {
        // Pure whitespace lines: compare full text.
        let old_text: String = old_tokens.iter().map(|t| t.text).collect();
        let new_text: String = new_tokens.iter().map(|t| t.text).collect();
        let sim = if old_text == new_text { 1.0 } else { 0.0 };
        if sim > 0.0 {
            common_old.fill(true);
            common_new.fill(true);
        }
        return (sim, common_old, common_new);
    }

    let onw_len = old_nw.len();
    let nnw_len = new_nw.len();
    let stride = nnw_len + 1;
    let mut dp = vec![0u16; (onw_len + 1) * stride];

    for i in 0..onw_len {
        let oi = old_nw[i];
        for j in 0..nnw_len {
            let nj = new_nw[j];
            if old_tokens[oi].text == new_tokens[nj].text {
                dp[(i + 1) * stride + (j + 1)] = dp[i * stride + j].saturating_add(1);
            } else {
                let from_top = dp[i * stride + (j + 1)];
                let from_left = dp[(i + 1) * stride + j];
                dp[(i + 1) * stride + (j + 1)] = from_top.max(from_left);
            }
        }
    }

    let mut i = onw_len;
    let mut j = nnw_len;

    while i > 0 && j > 0 {
        let oi = old_nw[i - 1];
        let nj = new_nw[j - 1];
        if old_tokens[oi].text == new_tokens[nj].text {
            common_old[oi] = true;
            common_new[nj] = true;
            i -= 1;
            j -= 1;
        } else if dp[(i - 1) * stride + j] >= dp[i * stride + (j - 1)] {
            i -= 1;
        } else {
            j -= 1;
        }
    }

    // Resolve whitespace tokens:
    // 1. Leading whitespace matches if identical.
    let old_first_nw = old_nw.first().copied().unwrap_or(n);
    let new_first_nw = new_nw.first().copied().unwrap_or(m);
    let old_lead: String = old_tokens[..old_first_nw].iter().map(|t| t.text).collect();
    let new_lead: String = new_tokens[..new_first_nw].iter().map(|t| t.text).collect();
    if !old_lead.is_empty() && old_lead == new_lead {
        common_old[..old_first_nw].fill(true);
        common_new[..new_first_nw].fill(true);
    }

    // 2. Trailing whitespace matches if identical.
    let old_last_nw = old_nw.last().copied().map(|idx| idx + 1).unwrap_or(0);
    let new_last_nw = new_nw.last().copied().map(|idx| idx + 1).unwrap_or(0);
    let old_trail: String = old_tokens[old_last_nw..].iter().map(|t| t.text).collect();
    let new_trail: String = new_tokens[new_last_nw..].iter().map(|t| t.text).collect();
    if !old_trail.is_empty() && old_trail == new_trail {
        common_old[old_last_nw..n].fill(true);
        common_new[new_last_nw..m].fill(true);
    }

    // 3. Intermediate whitespace: if both neighboring non-whitespace tokens are common,
    // the whitespace between them is considered common.
    for nw_slice in old_nw.windows(2) {
        let left = nw_slice[0];
        let right = nw_slice[1];
        if common_old[left] && common_old[right] {
            common_old[(left + 1)..right].fill(true);
        }
    }
    for nw_slice in new_nw.windows(2) {
        let left = nw_slice[0];
        let right = nw_slice[1];
        if common_new[left] && common_new[right] {
            common_new[(left + 1)..right].fill(true);
        }
    }

    // Similarity is based on non-whitespace character length overlap.
    let common_chars: usize = old_tokens
        .iter()
        .zip(common_old.iter())
        .filter_map(|(t, &c)| (c && !t.is_whitespace).then_some(t.text.len()))
        .sum();
    let old_chars: usize = old_tokens
        .iter()
        .filter_map(|t| (!t.is_whitespace).then_some(t.text.len()))
        .sum();
    let new_chars: usize = new_tokens
        .iter()
        .filter_map(|t| (!t.is_whitespace).then_some(t.text.len()))
        .sum();

    let total = old_chars + new_chars;
    let similarity = if total == 0 {
        0.0
    } else {
        (2.0 * common_chars as f32) / total as f32
    };

    (similarity, common_old, common_new)
}

/// Convert unshared token runs into byte ranges within the payload,
/// trimming boundary whitespace so that highlights wrap cleanly around actual changed words/tokens.
pub(super) fn extract_highlight_ranges(
    tokens: &[Token<'_>],
    common: &[bool],
    payload: &str,
) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        if !common[i] {
            let start_tok = i;
            while i < tokens.len() && !common[i] {
                i += 1;
            }
            let raw_start = tokens[start_tok].range.start;
            let raw_end = tokens[i - 1].range.end;
            if raw_start < raw_end && raw_end <= payload.len() {
                let text = &payload[raw_start..raw_end];
                if text.trim().is_empty() {
                    // When the difference is only whitespace (e.g. indentation change), highlight it directly.
                    ranges.push(raw_start..raw_end);
                } else {
                    let leading_ws = text.len() - text.trim_start().len();
                    let trailing_ws = text.len() - text.trim_end().len();
                    let trimmed_start = raw_start + leading_ws;
                    let trimmed_end = raw_end - trailing_ws;
                    if trimmed_start < trimmed_end {
                        ranges.push(trimmed_start..trimmed_end);
                    }
                }
            }
        } else {
            i += 1;
        }
    }
    ranges
}

pub(super) fn pair_block(
    raw: &str,
    lines: &mut [super::diff::DiffLine],
    deletions: &[usize],
    additions: &[usize],
) {
    let d_count = deletions.len();
    let a_count = additions.len();
    if d_count == 0 || a_count == 0 {
        return;
    }

    if d_count == 1 && a_count == 1 {
        let del_payload = &raw[lines[deletions[0]].payload.clone()];
        let add_payload = &raw[lines[additions[0]].payload.clone()];
        let del_tokens = tokenize(del_payload);
        let add_tokens = tokenize(add_payload);
        let (sim, common_del, common_add) = token_lcs(&del_tokens, &add_tokens);
        if sim >= MIN_SIMILARITY {
            lines[deletions[0]].highlights =
                extract_highlight_ranges(&del_tokens, &common_del, del_payload);
            lines[additions[0]].highlights =
                extract_highlight_ranges(&add_tokens, &common_add, add_payload);
        }
        return;
    }

    let del_tokens: Vec<Vec<Token<'_>>> = deletions
        .iter()
        .map(|&idx| tokenize(&raw[lines[idx].payload.clone()]))
        .collect();
    let add_tokens: Vec<Vec<Token<'_>>> = additions
        .iter()
        .map(|&idx| tokenize(&raw[lines[idx].payload.clone()]))
        .collect();

    // Fall back to 1:1 pairing if the block is very large to bound CPU time.
    if d_count.saturating_mul(a_count) > MAX_BLOCK_DP_CELLS {
        let pairs = d_count.min(a_count);
        for k in 0..pairs {
            let (sim, common_del, common_add) = token_lcs(&del_tokens[k], &add_tokens[k]);
            if sim >= MIN_SIMILARITY {
                let del_payload = &raw[lines[deletions[k]].payload.clone()];
                let add_payload = &raw[lines[additions[k]].payload.clone()];
                lines[deletions[k]].highlights =
                    extract_highlight_ranges(&del_tokens[k], &common_del, del_payload);
                lines[additions[k]].highlights =
                    extract_highlight_ranges(&add_tokens[k], &common_add, add_payload);
            }
        }
        return;
    }

    // Pairwise LCS similarities and alignment DP
    let mut sim = vec![0.0f32; d_count * a_count];
    let mut lcs_results = Vec::with_capacity(d_count * a_count);
    for i in 0..d_count {
        for j in 0..a_count {
            let res = token_lcs(&del_tokens[i], &add_tokens[j]);
            sim[i * a_count + j] = res.0;
            lcs_results.push(res);
        }
    }

    let stride = a_count + 1;
    let mut dp = vec![0.0f32; (d_count + 1) * stride];
    for i in 0..=d_count {
        for j in 0..=a_count {
            let current = dp[i * stride + j];
            if i < d_count {
                let next_idx = (i + 1) * stride + j;
                if current > dp[next_idx] {
                    dp[next_idx] = current;
                }
            }
            if j < a_count {
                let next_idx = i * stride + (j + 1);
                if current > dp[next_idx] {
                    dp[next_idx] = current;
                }
            }
            if i < d_count && j < a_count {
                let s = sim[i * a_count + j];
                if s >= MIN_SIMILARITY {
                    let next_idx = (i + 1) * stride + (j + 1);
                    let val = current + s;
                    if val > dp[next_idx] {
                        dp[next_idx] = val;
                    }
                }
            }
        }
    }

    // Backtrack optimal alignment
    let mut i = d_count;
    let mut j = a_count;
    while i > 0 && j > 0 {
        let current = dp[i * stride + j];
        let s = sim[(i - 1) * a_count + (j - 1)];
        if s >= MIN_SIMILARITY
            && (current - (dp[(i - 1) * stride + (j - 1)] + s)).abs() < 1e-4
        {
            let lcs_idx = (i - 1) * a_count + (j - 1);
            let (_, ref common_del, ref common_add) = lcs_results[lcs_idx];
            let del_payload = &raw[lines[deletions[i - 1]].payload.clone()];
            let add_payload = &raw[lines[additions[j - 1]].payload.clone()];
            lines[deletions[i - 1]].highlights =
                extract_highlight_ranges(&del_tokens[i - 1], common_del, del_payload);
            lines[additions[j - 1]].highlights =
                extract_highlight_ranges(&add_tokens[j - 1], common_add, add_payload);
            i -= 1;
            j -= 1;
        } else if (current - dp[(i - 1) * stride + j]).abs() < 1e-4 {
            i -= 1;
        } else {
            j -= 1;
        }
    }
}

pub(super) fn assign_word_diff_highlights(raw: &str, lines: &mut [super::diff::DiffLine]) {
    let mut i = 0;
    while i < lines.len() {
        if lines[i].kind == DiffLineKind::Deletion {
            let mut deletions = Vec::new();
            while i < lines.len() && lines[i].kind == DiffLineKind::Deletion {
                deletions.push(i);
                i += 1;
            }
            let mut additions = Vec::new();
            while i < lines.len() && lines[i].kind == DiffLineKind::Addition {
                additions.push(i);
                i += 1;
            }
            if !deletions.is_empty() && !additions.is_empty() {
                pair_block(raw, lines, &deletions, &additions);
            }
        } else {
            i += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::preview::DiffDocument;

    #[test]
    fn tokenizes_words_whitespace_and_punctuation() {
        let input = "Account profile and sign-in";
        let tokens = tokenize(input);
        let texts: Vec<&str> = tokens.iter().map(|t| t.text).collect();
        assert_eq!(
            texts,
            vec!["Account", " ", "profile", " ", "and", " ", "sign", "-", "in"]
        );
        assert!(!tokens[0].is_whitespace);
        assert!(tokens[1].is_whitespace);
        assert_eq!(tokens[2].range, 8..15);
    }

    #[test]
    fn highlights_inserted_and_deleted_words() {
        let old_line = "Account and sign-in information";
        let new_line = "Account profile and sign-in information";
        let old_tokens = tokenize(old_line);
        let new_tokens = tokenize(new_line);

        let (sim, common_old, common_new) = token_lcs(&old_tokens, &new_tokens);
        assert!(sim > 0.5);

        let old_hl = extract_highlight_ranges(&old_tokens, &common_old, old_line);
        let new_hl = extract_highlight_ranges(&new_tokens, &common_new, new_line);

        assert!(old_hl.is_empty());
        assert_eq!(new_hl.len(), 1);
        assert_eq!(&new_line[new_hl[0].clone()], "profile");
    }

    #[test]
    fn does_not_highlight_completely_dissimilar_lines() {
        let old_line = "completely different line here";
        let new_line = "something totally unrelated 12345";
        let old_tokens = tokenize(old_line);
        let new_tokens = tokenize(new_line);

        let (sim, _, _) = token_lcs(&old_tokens, &new_tokens);
        assert!(sim < MIN_SIMILARITY);
    }

    #[test]
    fn multiple_word_differences_in_code_line() {
        let old_line = "let old_name = foo(bar);";
        let new_line = "let new_name = foo(baz);";
        let old_tokens = tokenize(old_line);
        let new_tokens = tokenize(new_line);

        let (sim, common_old, common_new) = token_lcs(&old_tokens, &new_tokens);
        assert!(sim > MIN_SIMILARITY);

        let old_hl = extract_highlight_ranges(&old_tokens, &common_old, old_line);
        let new_hl = extract_highlight_ranges(&new_tokens, &common_new, new_line);

        assert_eq!(old_hl.len(), 2);
        assert_eq!(&old_line[old_hl[0].clone()], "old_name");
        assert_eq!(&old_line[old_hl[1].clone()], "bar");

        assert_eq!(new_hl.len(), 2);
        assert_eq!(&new_line[new_hl[0].clone()], "new_name");
        assert_eq!(&new_line[new_hl[1].clone()], "baz");
    }

    #[test]
    fn indentation_change_highlights_whitespace() {
        let old_line = "    value";
        let new_line = "        value";
        let old_tokens = tokenize(old_line);
        let new_tokens = tokenize(new_line);

        let (sim, _common_old, common_new) = token_lcs(&old_tokens, &new_tokens);
        assert!(sim > 0.4);

        let new_hl = extract_highlight_ranges(&new_tokens, &common_new, new_line);
        assert_eq!(new_hl.len(), 1);
        assert_eq!(&new_line[new_hl[0].clone()], "        ");
    }

    #[test]
    fn assigns_word_diff_to_diff_document() {
        let diff = concat!(
            "diff --git a/doc.md b/doc.md\n",
            "--- a/doc.md\n",
            "+++ b/doc.md\n",
            "@@ -1 +1 @@\n",
            "-  - **Account and sign-in information** is deleted or de-identified when you delete your account.\n",
            "+  - **Account profile and sign-in information** is removed when account deletion completes.\n",
        );
        let document = DiffDocument::parse(diff.to_owned());
        // Row 0 is the hunk header (@@ -1 +1 @@) when show_headers is false.
        // Row 1 is the deletion line (-  - **Account...).
        // Row 2 is the addition line (+  - **Account profile...).
        let del_highlights = document.display_highlights(1, false);
        let add_highlights = document.display_highlights(2, false);

        assert!(!del_highlights.is_empty());
        assert!(!add_highlights.is_empty());

        let del_line = document.display_line(1, false).unwrap();
        let add_line = document.display_line(2, false).unwrap();

        // Check that "profile" is in the added highlights (line starts with '+')
        let add_payload = &add_line[1..];
        let highlighted_words: Vec<&str> = add_highlights
            .iter()
            .map(|r| &add_payload[r.clone()])
            .collect();
        assert!(highlighted_words.contains(&"profile"));
        assert!(highlighted_words.contains(&"removed"));
        assert!(highlighted_words.contains(&"deletion completes"));

        let del_payload = &del_line[1..];
        let del_highlighted_words: Vec<&str> = del_highlights
            .iter()
            .map(|r| &del_payload[r.clone()])
            .collect();
        assert!(del_highlighted_words.contains(&"deleted or de-identified"));
        assert!(del_highlighted_words.contains(&"you delete your"));

        // Check that "Account" and "when" are NOT highlighted in either
        for hl in add_highlights {
            assert_ne!(&add_payload[hl.clone()], "Account");
            assert_ne!(&add_payload[hl.clone()], "when");
        }
        for hl in del_highlights {
            assert_ne!(&del_payload[hl.clone()], "Account");
            assert_ne!(&del_payload[hl.clone()], "when");
        }
    }
}
