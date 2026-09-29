// 利用者が明示操作したときだけ、ダウンロードフォルダのモデル候補を読み取り専用で探す。
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context, Result};
use serde::Serialize;

const MAX_TOP_LEVEL: usize = 300;
const MAX_CANDIDATES: usize = 30;
const MAX_FOLDER_ENTRIES: usize = 4_000;
const MAX_DEPTH: usize = 8;

#[derive(Debug, PartialEq, Serialize)]
pub struct Candidate {
    pub kind: &'static str,
    pub path: String,
    pub name: String,
}

#[derive(Default)]
struct ModelCounts {
    mmd: usize,
    live2d: usize,
}

impl ModelCounts {
    fn add(&mut self, name: &str) {
        let lower = name.to_ascii_lowercase();
        if lower.ends_with(".pmx") {
            self.mmd += 1;
        } else if lower.ends_with(".model3.json") {
            self.live2d += 1;
        }
    }

    fn kind(&self) -> Option<&'static str> {
        match (self.mmd, self.live2d) {
            (1, 0) => Some("mmd"),
            (0, 1) => Some("live2d"),
            _ => None,
        }
    }
}

fn folder_models(
    path: &Path,
    root: &Path,
    depth: usize,
    seen: &mut usize,
    counts: &mut ModelCounts,
) -> Result<()> {
    if depth > MAX_DEPTH || *seen > MAX_FOLDER_ENTRIES {
        return Ok(());
    }
    for entry in std::fs::read_dir(path)
        .with_context(|| format!("フォルダを読めません: {}", path.display()))?
    {
        let entry = entry?;
        *seen += 1;
        if *seen > MAX_FOLDER_ENTRIES {
            return Ok(());
        }
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            continue;
        }
        if !entry.path().canonicalize()?.starts_with(root) {
            continue;
        }
        if file_type.is_dir() {
            folder_models(&entry.path(), root, depth + 1, seen, counts)?;
        } else if file_type.is_file() {
            counts.add(&entry.file_name().to_string_lossy());
        }
        if counts.mmd + counts.live2d > 1 {
            break;
        }
    }
    Ok(())
}

fn zip_models(path: &Path) -> Option<ModelCounts> {
    let file = std::fs::File::open(path).ok()?;
    let mut archive = zip::ZipArchive::new(file).ok()?;
    if archive.len() > MAX_FOLDER_ENTRIES {
        return None;
    }
    let mut counts = ModelCounts::default();
    for index in 0..archive.len() {
        let entry = archive.by_index(index).ok()?;
        if !entry.is_dir() {
            counts.add(entry.name());
        }
        if counts.mmd + counts.live2d > 1 {
            break;
        }
    }
    Some(counts)
}

fn model_kind(path: &Path) -> Option<&'static str> {
    let file_type = std::fs::symlink_metadata(path).ok()?.file_type();
    if file_type.is_symlink() {
        return None;
    }
    if file_type.is_dir() {
        let mut counts = ModelCounts::default();
        let root = path.canonicalize().ok()?;
        folder_models(path, &root, 0, &mut 0, &mut counts).ok()?;
        return counts.kind();
    }
    let name = path.file_name()?.to_string_lossy().to_ascii_lowercase();
    if name.ends_with(".pmx") {
        return Some("mmd");
    }
    if name.ends_with(".zip") {
        return zip_models(path)?.kind();
    }
    None
}

pub fn discover(downloads: &Path) -> Result<Vec<Candidate>> {
    let mut entries: Vec<(SystemTime, PathBuf)> = std::fs::read_dir(downloads)
        .with_context(|| format!("ダウンロードフォルダを読めません: {}", downloads.display()))?
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            let metadata = std::fs::symlink_metadata(&path).ok()?;
            if metadata.file_type().is_symlink() {
                return None;
            }
            Some((metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH), path))
        })
        .collect();
    entries.sort_by_key(|entry| std::cmp::Reverse(entry.0));
    let mut out = Vec::new();
    for (_, path) in entries.into_iter().take(MAX_TOP_LEVEL) {
        let Some(kind) = model_kind(&path) else {
            continue;
        };
        let name = if path.is_dir() {
            path.file_name()
        } else {
            path.file_stem()
        }
        .map(|value| value.to_string_lossy().to_string())
        .unwrap_or_default();
        out.push(Candidate {
            kind,
            path: path.to_string_lossy().to_string(),
            name,
        });
        if out.len() == MAX_CANDIDATES {
            break;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::discover;
    use std::io::Write;

    #[test]
    fn 単一モデルだけを候補とし複数モデルや無関係なファイルは除く() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let live2d = root.join("ひより").join("runtime");
        std::fs::create_dir_all(&live2d).unwrap();
        std::fs::write(live2d.join("Hiyori.model3.json"), b"{}").unwrap();
        let multi = root.join("複数モデル");
        std::fs::create_dir_all(&multi).unwrap();
        std::fs::write(multi.join("a.pmx"), b"").unwrap();
        std::fs::write(multi.join("b.pmx"), b"").unwrap();
        std::fs::write(root.join("readme.txt"), b"").unwrap();
        let zip_path = root.join("MMD.zip");
        let file = std::fs::File::create(&zip_path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file("model/a.pmx", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"pmx").unwrap();
        zip.finish().unwrap();

        let found = discover(root).unwrap();
        assert_eq!(found.len(), 2);
        assert!(found
            .iter()
            .any(|item| item.kind == "live2d" && item.name == "ひより"));
        assert!(found
            .iter()
            .any(|item| item.kind == "mmd" && item.name == "MMD"));
    }
}
