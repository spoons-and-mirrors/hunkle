use super::*;

pub(crate) fn log(root: &Path) -> Result<Vec<Commit>> {
    read_log(
        root,
        &[
            "--date-order",
            "--ignore-missing",
            "--max-count=5001",
            "--branches",
            "--remotes",
            "--tags",
            "HEAD",
        ],
    )
}

pub(crate) fn branch_history(root: &Path) -> Result<Vec<Commit>> {
    read_log(
        root,
        &[
            "--date-order",
            "--ignore-missing",
            "--max-count=200",
            "HEAD",
        ],
    )
}

fn read_log(root: &Path, revisions: &[&str]) -> Result<Vec<Commit>> {
    let format = "--format=%H%x00%P%x00%D%x00%an%x00%ad%x00%s%x00%B%x00";
    let mut args = vec![
        "log",
        format,
        "--date=format:%Y-%m-%d %H:%M",
        "--decorate=short",
    ];
    args.extend_from_slice(revisions);
    let output = run(root, &args)?;

    if !output.status.success() {
        let stderr = clean_stderr(&output);
        if stderr.contains("does not have any commits yet")
            || stderr.contains("bad revision 'HEAD'")
            || stderr.contains("ambiguous argument 'HEAD'")
        {
            return Ok(Vec::new());
        }
        bail!("{stderr}");
    }

    Ok(parse_log(&output.stdout))
}

pub(super) fn parse_log(bytes: &[u8]) -> Vec<Commit> {
    bytes
        .split(|byte| *byte == 0)
        .collect::<Vec<_>>()
        .chunks_exact(7)
        .map(|fields| {
            let text = |field: &[u8]| String::from_utf8_lossy(field).into_owned();
            let decorations = text(fields[2]);
            Commit {
                oid: text(trim_ascii(fields[0])),
                parents: text(fields[1])
                    .split_whitespace()
                    .map(str::to_owned)
                    .collect(),
                refs: decorations
                    .split(", ")
                    .filter(|name| !name.is_empty())
                    .map(str::to_owned)
                    .collect(),
                author: text(fields[3]),
                date: compact_commit_date(text(fields[4])),
                subject: text(fields[5]),
                message: text(fields[6]),
                graph: Vec::new(),
            }
        })
        .collect()
}

fn compact_commit_date(date: String) -> String {
    let bytes = date.as_bytes();
    if !date.is_ascii()
        || bytes.len() != 16
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b' '
        || bytes[13] != b':'
    {
        return date;
    }
    let month = match &date[5..7] {
        "01" => "Jan",
        "02" => "Feb",
        "03" => "Mar",
        "04" => "Apr",
        "05" => "May",
        "06" => "Jun",
        "07" => "Jul",
        "08" => "Aug",
        "09" => "Sep",
        "10" => "Oct",
        "11" => "Nov",
        "12" => "Dec",
        _ => return date,
    };
    format!("{}{month} {}", &date[8..10], &date[11..16])
}
