use super::*;

#[test]
fn wraps_file_summaries_with_a_bounded_height() {
    let files = ["src/one.rs", "src/two.rs", "src/three.rs", "src/four.rs"].map(RepoPath::from);
    let lines = wrapped_file_summary(&files, 12, 3);
    assert_eq!(lines.len(), 3);
    assert!(lines.last().unwrap().ends_with('…'));
    assert!(
        lines
            .iter()
            .all(|line| UnicodeWidthStr::width(line.as_str()) <= 12)
    );

    let summary = DiffSummary {
        files: files.to_vec(),
        files_truncated: false,
        additions: 1,
        deletions: 1,
    };
    assert_eq!(diff_summary_height(Some(&summary), 19, false, 8), 3);
    assert!(diff_summary_height(Some(&summary), 19, true, 8) > 3);
}
