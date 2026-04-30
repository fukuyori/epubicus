use super::*;

pub(super) fn write_json_pretty_atomic<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let tmp = tmp_path(path);
    let data = serde_json::to_vec_pretty(value).context("failed to serialize JSON")?;
    fs::write(&tmp, data).with_context(|| format!("failed to write {}", tmp.display()))?;
    fs::rename(&tmp, path).with_context(|| format!("failed to commit {}", path.display()))?;
    Ok(())
}

pub(super) fn read_json_file<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T> {
    let data = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    serde_json::from_slice(&data).with_context(|| format!("failed to parse {}", path.display()))
}

pub(super) fn read_optional_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<Option<T>> {
    if path.exists() {
        read_json_file(path).map(Some)
    } else {
        Ok(None)
    }
}

pub(super) fn read_jsonl_file<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<Vec<T>> {
    let text =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    text.lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(idx, line)| {
            serde_json::from_str(line)
                .with_context(|| format!("failed to parse {} line {}", path.display(), idx + 1))
        })
        .collect()
}

pub(super) fn count_jsonl_lines(path: &Path) -> Result<usize> {
    if !path.exists() {
        return Ok(0);
    }
    Ok(fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?
        .lines()
        .filter(|line| !line.trim().is_empty())
        .count())
}

pub(super) fn write_jsonl_atomic<'a, T, I>(path: &Path, values: I) -> Result<()>
where
    T: Serialize + 'a,
    I: IntoIterator<Item = &'a T>,
{
    let tmp = tmp_path(path);
    let mut file =
        File::create(&tmp).with_context(|| format!("failed to create {}", tmp.display()))?;
    for value in values {
        serde_json::to_writer(&mut file, value).context("failed to serialize JSONL")?;
        writeln!(file).context("failed to write JSONL newline")?;
    }
    file.flush()
        .with_context(|| format!("failed to flush {}", tmp.display()))?;
    fs::rename(&tmp, path).with_context(|| format!("failed to commit {}", path.display()))?;
    Ok(())
}

pub(super) fn tmp_path(path: &Path) -> PathBuf {
    path.with_extension("tmp")
}
