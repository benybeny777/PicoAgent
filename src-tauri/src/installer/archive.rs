// archive - 用途の上限を保ったZIP/tar.gz展開。書庫内と保存先のリンクを拒否する。
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use anyhow::{bail, Context, Result};

use super::{fs, Limits};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    Fresh,
    Merge,
}
#[derive(Clone, Copy)]
pub(crate) enum NameEncoding {
    Utf8,
    Utf8OrCp932,
}
#[derive(Clone, Copy)]
pub(crate) enum Permissions {
    Plain,
    Executable,
}
#[derive(Clone, Copy)]
pub(crate) struct ExtractOptions {
    pub mode: Mode,
    pub name_encoding: NameEncoding,
    pub permissions: Permissions,
}

fn decode_name(raw: &[u8], encoding: NameEncoding) -> Result<String> {
    match std::str::from_utf8(raw) {
        Ok(name) => Ok(name.to_owned()),
        Err(_) if matches!(encoding, NameEncoding::Utf8OrCp932) => {
            let (name, _, errors) = encoding_rs::SHIFT_JIS.decode(raw);
            anyhow::ensure!(!errors, "書庫のファイル名をUTF-8またはCP932で読めません");
            Ok(name.into_owned())
        }
        Err(error) => Err(error).context("書庫のファイル名がUTF-8ではありません"),
    }
}

fn relative_name(name: &str) -> Result<PathBuf> {
    let normalized = name.replace('\\', "/");
    anyhow::ensure!(
        !normalized.contains([':', '\0']) && !normalized.starts_with('/'),
        "書庫に安全でないパスが含まれています: {name}"
    );
    let mut path = PathBuf::new();
    for part in Path::new(&normalized).components() {
        match part {
            Component::CurDir => {}
            Component::Normal(value) => {
                #[cfg(windows)]
                {
                    let value = value.to_str().context("書庫の名前をUTF-8で読めません")?;
                    let stem = value.split('.').next().unwrap_or("").to_ascii_uppercase();
                    anyhow::ensure!(
                        !value.ends_with(['.', ' '])
                            && !value.contains(['<', '>', '"', '|', '?', '*'])
                            && !value.chars().any(|ch| ch < ' ')
                            && ![
                                "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5",
                                "COM6", "COM7", "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4",
                                "LPT5", "LPT6", "LPT7", "LPT8", "LPT9"
                            ]
                            .contains(&stem.as_str()),
                        "Windowsで安全でない書庫名です: {name}"
                    );
                }
                path.push(value);
            }
            _ => bail!("書庫に .. やドライブ指定は使えません: {name}"),
        }
    }
    anyhow::ensure!(!path.as_os_str().is_empty(), "書庫のパスが空です");
    Ok(path)
}

fn name_key(path: &Path) -> String {
    let name = path.to_string_lossy().replace('\\', "/");
    if cfg!(windows) {
        name.to_lowercase()
    } else {
        name
    }
}

#[derive(Default)]
struct Manifest {
    names: HashMap<String, bool>,
    total: u64,
    count: usize,
}
impl Manifest {
    fn add(&mut self, relative: &Path, directory: bool, size: u64, limits: Limits) -> Result<()> {
        self.count = self
            .count
            .checked_add(1)
            .context("書庫の項目数が多すぎます")?;
        anyhow::ensure!(
            self.count <= limits.max_entries,
            "書庫の項目が用途の上限を超えています"
        );
        anyhow::ensure!(
            self.names.insert(name_key(relative), directory).is_none(),
            "書庫内の名前が重複しています: {}",
            relative.display()
        );
        anyhow::ensure!(
            !directory || size == 0,
            "書庫のフォルダ項目に本文は置けません"
        );
        if !directory {
            anyhow::ensure!(
                size <= fs::file_limit(limits, relative),
                "書庫のファイルが用途の上限を超えています"
            );
            self.total = self
                .total
                .checked_add(size)
                .context("書庫のサイズが多すぎます")?;
            anyhow::ensure!(
                self.total <= limits.max_total_bytes,
                "書庫の合計サイズが用途の上限を超えています"
            );
        }
        Ok(())
    }
    fn validate(&self) -> Result<()> {
        for name in self.names.keys() {
            let mut parent = Path::new(name).parent();
            while let Some(path) = parent {
                anyhow::ensure!(
                    self.names.get(&name_key(path)) != Some(&false),
                    "書庫内でファイルとフォルダの名前が衝突しています: {name}"
                );
                parent = path.parent();
            }
        }
        Ok(())
    }
}

/// 外部展開器の一覧も同じ正本で検査する。リンク等の項目型は呼出元で拒否してから渡す。
pub(crate) fn validate_external_entries(
    entries: &[(String, bool, u64)],
    limits: Limits,
) -> Result<()> {
    anyhow::ensure!(
        entries.len() <= limits.max_entries,
        "書庫の項目が用途の上限を超えています"
    );
    let mut manifest = Manifest::default();
    for (name, directory, size) in entries {
        if *directory && matches!(name.as_str(), "." | "./") {
            anyhow::ensure!(*size == 0, "書庫のルートフォルダに本文は置けません");
            manifest.count += 1;
            continue;
        }
        let relative = relative_name(name)?;
        manifest.add(&relative, *directory, *size, limits)?;
    }
    manifest.validate()
}

fn prepare_destination(destination: &Path, mode: Mode) -> Result<PathBuf> {
    let root = fs::checked_directory(destination)?;
    if mode == Mode::Fresh {
        anyhow::ensure!(
            std::fs::read_dir(&root)?.next().is_none(),
            "新規展開先は空のフォルダを指定してください。既存の配置は変更していません"
        );
    }
    Ok(root)
}

fn zip_entry<R: Read>(
    entry: &zip::read::ZipFile<'_, R>,
    encoding: NameEncoding,
) -> Result<(PathBuf, bool, u32)> {
    let relative = relative_name(&decode_name(entry.name_raw(), encoding)?)?;
    let mode = entry.unix_mode().unwrap_or(0);
    let kind = mode & 0o170000;
    anyhow::ensure!(
        matches!(kind, 0 | 0o100000 | 0o040000),
        "書庫内にリンク・特殊ファイルは置けません"
    );
    let directory = entry.is_dir();
    anyhow::ensure!(
        kind == 0 || (directory && kind == 0o040000) || (!directory && kind == 0o100000),
        "書庫の項目種別が不正です"
    );
    Ok((relative, directory, mode))
}

struct Extraction<'a> {
    root: &'a Path,
    limits: Limits,
    options: ExtractOptions,
    total: u64,
}
impl Extraction<'_> {
    fn put(
        &mut self,
        reader: &mut impl Read,
        relative: &Path,
        size: u64,
        mode: u32,
        directory: bool,
    ) -> Result<()> {
        let target = fs::destination(self.root, relative)?;
        if directory {
            fs::checked_directory(&target)?;
            return Ok(());
        }
        let limit = self
            .limits
            .max_total_bytes
            .saturating_sub(self.total)
            .min(fs::file_limit(self.limits, &target));
        let existing = target.try_exists()?;
        let copied = fs::write_file(
            reader,
            &target,
            limit,
            size,
            self.options.mode == Mode::Merge,
        )?;
        self.total = self
            .total
            .checked_add(copied)
            .context("展開サイズが多すぎます")?;
        // Mergeの同内容ファイルは属性も変更しない。新規公開したものだけを安全な権限へそろえる。
        if !existing {
            fs::set_safe_permissions(
                &target,
                matches!(self.options.permissions, Permissions::Executable) && mode & 0o111 != 0,
            )?;
        }
        Ok(())
    }
}

#[derive(Debug)]
struct ZipDirectory {
    start: u64,
    end: u64,
    count: usize,
}

fn zip_count(count: u64, limits: Limits) -> Result<usize> {
    anyhow::ensure!(
        count <= limits.max_entries as u64,
        "書庫の項目が用途の上限を超えています"
    );
    Ok(usize::try_from(count)?)
}

// zip 8.6は詳細解析失敗で別EOCDへ再探索する。各固定終端ヘッダを索引確保前に検査する。
// 詳細解析だけが対象で、通常の本文読込みには検査を適用しない。
struct ZipIndexReader<R> {
    inner: R,
    checking: Arc<AtomicBool>,
    limits: Limits,
    position: u64,
    last_read: Option<std::ops::Range<u64>>,
    candidate_read: bool,
    #[cfg(test)]
    rejected: Arc<AtomicBool>,
}
impl<R: Read> Read for ZipIndexReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let candidate = std::mem::take(&mut self.candidate_read);
        let count = if self.checking.load(Ordering::Relaxed)
            && candidate
            && matches!(buffer.len(), 22 | 56)
        {
            self.inner.read_exact(buffer)?;
            let count = if buffer.starts_with(b"PK\x05\x06") {
                let count = u16::from_le_bytes(zip_number(buffer, 10));
                (count != u16::MAX).then_some(u64::from(count))
            } else if buffer.starts_with(b"PK\x06\x06") {
                Some(u64::from_le_bytes(zip_number(buffer, 32)))
            } else {
                None
            };
            if let Some(count) = count {
                if let Err(error) = zip_count(count, self.limits) {
                    #[cfg(test)]
                    self.rejected.store(true, Ordering::Relaxed);
                    return Err(std::io::Error::other(error));
                }
            }
            buffer.len()
        } else {
            self.inner.read(buffer)?
        };
        self.last_read = Some(self.position..self.position + count as u64);
        self.position += count as u64;
        Ok(count)
    }
}
impl<R: Seek> Seek for ZipIndexReader<R> {
    fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
        let position = self.inner.seek(position)?;
        // MagicFinderは探索窓/4文字の仮読み後、見つかった署名位置へ戻って固定ヘッダを読む。
        // コメント・拡張・extra等の連続読込はこの条件に入れず、任意データを終端と誤認しない。
        self.candidate_read = self
            .last_read
            .as_ref()
            .is_some_and(|range| range.contains(&position));
        self.position = position;
        Ok(position)
    }
}

fn zip_number<const N: usize>(bytes: &[u8], offset: usize) -> [u8; N] {
    bytes[offset..offset + N]
        .try_into()
        .expect("固定ヘッダの範囲")
}

fn zip_block<const N: usize>(reader: &mut (impl Read + Seek), offset: u64) -> Result<[u8; N]> {
    reader.seek(SeekFrom::Start(offset))?;
    let mut bytes = [0; N];
    reader.read_exact(&mut bytes)?;
    Ok(bytes)
}

// 固定するのはバッファだけ。前置データやZIP64拡張領域に互換性を下げる容量上限は置かない。
fn zip_signature(
    reader: &mut (impl Read + Seek),
    signature: [u8; 4],
    begin: u64,
    end: u64,
    backwards: bool,
) -> Result<Option<u64>> {
    let mut edge = if backwards { end } else { begin };
    let mut buffer = [0; 65536];
    while if backwards {
        edge >= begin.saturating_add(4)
    } else {
        edge.saturating_add(4) <= end
    } {
        let start = if backwards {
            edge.saturating_sub(buffer.len() as u64).max(begin)
        } else {
            edge
        };
        let stop = if backwards {
            edge
        } else {
            end.min(start.saturating_add(buffer.len() as u64))
        };
        let length = (stop - start) as usize;
        reader.seek(SeekFrom::Start(start))?;
        reader.read_exact(&mut buffer[..length])?;
        let found = if backwards {
            buffer[..length].windows(4).rposition(|v| v == signature)
        } else {
            buffer[..length].windows(4).position(|v| v == signature)
        };
        if let Some(index) = found {
            return Ok(Some(start + index as u64));
        }
        if backwards {
            if start == begin {
                break;
            }
            edge = start + 3;
        } else {
            if stop == end {
                break;
            }
            edge = stop - 3;
        }
    }
    Ok(None)
}

// 初期索引の確保前に宣言件数と生中央一覧を検査する。名前・サイズ・属性はzipに任せる。
fn zip_directory(reader: &mut (impl Read + Seek), limits: Limits) -> Result<ZipDirectory> {
    let file_end = reader.seek(SeekFrom::End(0))?;
    let mut search_end = file_end;
    let (eocd_at, eocd) = loop {
        let at = zip_signature(reader, *b"PK\x05\x06", 0, search_end, true)?
            .context("ZIPの終端一覧がありません")?;
        search_end = at + 3;
        let Ok(block) = zip_block::<22>(reader, at) else {
            continue;
        };
        let comment = u64::from(u16::from_le_bytes(zip_number(&block, 20)));
        if at
            .checked_add(22 + comment)
            .is_some_and(|end| end <= file_end)
        {
            break (at, block);
        }
    };
    let disk = u16::from_le_bytes(zip_number(&eocd, 4));
    let cd_disk = u16::from_le_bytes(zip_number(&eocd, 6));
    let disk_count = u16::from_le_bytes(zip_number(&eocd, 8));
    let count32 = u16::from_le_bytes(zip_number(&eocd, 10));
    let size32 = u32::from_le_bytes(zip_number(&eocd, 12));
    let offset32 = u32::from_le_bytes(zip_number(&eocd, 16));
    anyhow::ensure!(
        disk == 0 && cd_disk == 0,
        "分割ディスクZIPは対応していません"
    );
    let needs64 =
        count32 == u16::MAX || disk_count == u16::MAX || size32 == u32::MAX || offset32 == u32::MAX;
    let locator = if eocd_at >= 20 {
        zip_block::<20>(reader, eocd_at - 20)?
    } else {
        [0; 20]
    };
    let (count, size, offset, directory_end) = if locator.starts_with(b"PK\x06\x07") {
        anyhow::ensure!(
            u32::from_le_bytes(zip_number(&locator, 4)) == 0
                && u32::from_le_bytes(zip_number(&locator, 16)) == 1,
            "分割ディスクZIP64は対応していません"
        );
        let locator_at = eocd_at - 20;
        let relative = u64::from_le_bytes(zip_number(&locator, 8));
        let mut begin = relative;
        let (at, block) = loop {
            let at = zip_signature(reader, *b"PK\x06\x06", begin, locator_at, false)?
                .context("ZIP64の終端一覧がありません")?;
            begin = at + 1;
            let Ok(block) = zip_block::<56>(reader, at) else {
                continue;
            };
            let size = u64::from_le_bytes(zip_number(&block, 4));
            if size >= 44
                && at.checked_add(12).and_then(|v| v.checked_add(size)) == Some(locator_at)
            {
                break (at, block);
            }
        };
        anyhow::ensure!(
            u32::from_le_bytes(zip_number(&block, 16)) == 0
                && u32::from_le_bytes(zip_number(&block, 20)) == 0,
            "分割ディスクZIP64は対応していません"
        );
        let count = u64::from_le_bytes(zip_number(&block, 32));
        anyhow::ensure!(
            u64::from_le_bytes(zip_number(&block, 24)) == count,
            "ZIP64の項目数が一致しません"
        );
        anyhow::ensure!(
            (count32 == u16::MAX || u64::from(count32) == count)
                && (disk_count == u16::MAX || u64::from(disk_count) == count),
            "ZIPとZIP64の項目数が一致しません"
        );
        let size = u64::from_le_bytes(zip_number(&block, 40));
        let offset = u64::from_le_bytes(zip_number(&block, 48));
        let start = at
            .checked_sub(size)
            .context("ZIP64の中央一覧が範囲外です")?;
        anyhow::ensure!(
            at.checked_sub(relative) == start.checked_sub(offset),
            "ZIP64の前置位置が一致しません"
        );
        (count, size, offset, at)
    } else {
        anyhow::ensure!(!needs64, "ZIP64の終端位置が欠損しています");
        anyhow::ensure!(disk_count == count32, "ZIPの項目数が一致しません");
        (
            u64::from(count32),
            u64::from(size32),
            u64::from(offset32),
            eocd_at,
        )
    };
    let count = zip_count(count, limits)?;
    let start = directory_end
        .checked_sub(size)
        .context("ZIPの中央一覧が範囲外です")?;
    anyhow::ensure!(start >= offset, "ZIPの中央一覧位置が不正です");
    let directory = ZipDirectory {
        start,
        end: directory_end,
        count,
    };
    validate_zip_directory(reader, &directory, limits)?;
    Ok(directory)
}

// 生一覧は宣言と同じ件数でなければならず、項目長も中央一覧の範囲から出さない。
fn validate_zip_directory(
    reader: &mut (impl Read + Seek),
    directory: &ZipDirectory,
    limits: Limits,
) -> Result<()> {
    reader.seek(SeekFrom::Start(directory.start))?;
    let mut count = 0usize;
    while reader.stream_position()? < directory.end {
        anyhow::ensure!(
            directory.end - reader.stream_position()? >= 4,
            "ZIPの中央一覧が途中で切れています"
        );
        let mut signature = [0; 4];
        reader.read_exact(&mut signature)?;
        let skip = match u32::from_le_bytes(signature) {
            0x0201_4b50 => {
                count = count.checked_add(1).context("書庫の項目数が多すぎます")?;
                anyhow::ensure!(
                    count <= limits.max_entries,
                    "書庫の項目が用途の上限を超えています"
                );
                let mut header = [0; 42];
                anyhow::ensure!(
                    directory.end - reader.stream_position()? >= 42,
                    "ZIPの中央一覧が途中で切れています"
                );
                reader.read_exact(&mut header)?;
                [24, 26, 28]
                    .into_iter()
                    .map(|offset| {
                        u64::from(u16::from_le_bytes([header[offset], header[offset + 1]]))
                    })
                    .sum::<u64>()
            }
            // 中央一覧の電子署名はファイル項目ではない。
            0x0505_4b50 => {
                anyhow::ensure!(
                    directory.end - reader.stream_position()? >= 2,
                    "ZIPの中央署名が途中で切れています"
                );
                let mut length = [0; 2];
                reader.read_exact(&mut length)?;
                u64::from(u16::from_le_bytes(length))
            }
            _ => bail!("ZIPの中央一覧が壊れています"),
        };
        let next = reader
            .stream_position()?
            .checked_add(skip)
            .context("ZIPの中央一覧が大きすぎます")?;
        anyhow::ensure!(next <= directory.end, "ZIPの中央一覧が途中で切れています");
        reader.seek(SeekFrom::Start(next))?;
    }
    anyhow::ensure!(
        count == directory.count,
        "ZIPの宣言と生中央一覧の項目数が一致しません"
    );
    Ok(())
}

pub(crate) fn extract_zip(
    archive: &Path,
    destination: &Path,
    limits: Limits,
    options: ExtractOptions,
) -> Result<()> {
    let mut input = std::fs::File::open(archive)?;
    let directory = zip_directory(&mut input, limits)?;
    let checking = Arc::new(AtomicBool::new(true));
    let reader = ZipIndexReader {
        inner: input,
        checking: checking.clone(),
        limits,
        position: 0,
        last_read: None,
        candidate_read: false,
        #[cfg(test)]
        rejected: Arc::new(AtomicBool::new(false)),
    };
    let mut zip = zip::ZipArchive::new(reader)?;
    checking.store(false, Ordering::Relaxed);
    anyhow::ensure!(
        zip.central_directory_start() == directory.start && zip.len() == directory.count,
        "ZIPの中央一覧に同じ生名前の重複や未公開の項目があります"
    );
    anyhow::ensure!(
        zip.len() <= limits.max_entries,
        "書庫の項目が用途の上限を超えています"
    );
    let mut manifest = Manifest::default();
    for index in 0..zip.len() {
        let entry = zip.by_index(index)?;
        let (relative, directory, _) = zip_entry(&entry, options.name_encoding)?;
        manifest.add(&relative, directory, entry.size(), limits)?;
    }
    manifest.validate()?;
    let root = prepare_destination(destination, options.mode)?;
    let mut extraction = Extraction {
        root: &root,
        limits,
        options,
        total: 0,
    };
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index)?;
        let (relative, directory, mode) = zip_entry(&entry, options.name_encoding)?;
        let size = entry.size();
        extraction.put(&mut entry, &relative, size, mode, directory)?;
    }
    Ok(())
}

type TarReader = tar::Archive<std::io::Take<flate2::read::GzDecoder<std::fs::File>>>;

fn tar_reader(archive: &Path, limits: Limits) -> Result<TarReader> {
    // 書庫ヘッダ・長い名前の補助項目にも有限の読み込み予算を設ける。
    let metadata_budget = u64::try_from(limits.max_entries)?
        .checked_mul(65536)
        .context("書庫の項目上限が大きすぎます")?;
    let budget = limits
        .max_total_bytes
        .checked_add(metadata_budget)
        .and_then(|value| value.checked_add(1025))
        .context("書庫の読込上限が大きすぎます")?;
    Ok(tar::Archive::new(
        flate2::read::GzDecoder::new(std::fs::File::open(archive)?).take(budget),
    ))
}

fn tar_entry<R: Read>(
    entry: &tar::Entry<'_, R>,
    encoding: NameEncoding,
) -> Result<(PathBuf, bool, u32)> {
    let kind = entry.header().entry_type();
    anyhow::ensure!(
        kind.is_file() || kind.is_dir(),
        "tar内にリンク・特殊ファイル・疎なファイルは置けません"
    );
    let name = decode_name(&entry.path_bytes(), encoding)?;
    Ok((relative_name(&name)?, kind.is_dir(), entry.header().mode()?))
}

pub(crate) fn extract_tar_gz(
    archive: &Path,
    destination: &Path,
    limits: Limits,
    options: ExtractOptions,
) -> Result<()> {
    let mut tar = tar_reader(archive, limits)?;
    let mut manifest = Manifest::default();
    for entry in tar.entries()? {
        let entry = entry?;
        let (relative, directory, _) = tar_entry(&entry, options.name_encoding)?;
        manifest.add(&relative, directory, entry.size(), limits)?;
    }
    manifest.validate()?;
    let mut reader = tar.into_inner();
    std::io::copy(&mut reader, &mut std::io::sink())?;
    anyhow::ensure!(reader.limit() > 0, "書庫ヘッダの読込が上限を超えています");
    let root = prepare_destination(destination, options.mode)?;
    let mut tar = tar_reader(archive, limits)?;
    let mut extraction = Extraction {
        root: &root,
        limits,
        options,
        total: 0,
    };
    for entry in tar.entries()? {
        let mut entry = entry?;
        let (relative, directory, mode) = tar_entry(&entry, options.name_encoding)?;
        let size = entry.size();
        extraction.put(&mut entry, &relative, size, mode, directory)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const LIMITS: Limits = Limits {
        max_entries: 10,
        max_total_bytes: 32,
        max_file_bytes: 16,
        file_limit: None,
    };
    const OPTIONS: ExtractOptions = ExtractOptions {
        mode: Mode::Fresh,
        name_encoding: NameEncoding::Utf8,
        permissions: Permissions::Executable,
    };

    fn zip_file(root: &Path, files: &[(&str, &[u8])]) -> PathBuf {
        zip_file_options(root, files, zip::write::SimpleFileOptions::default())
    }

    fn zip_file_options(
        root: &Path,
        files: &[(&str, &[u8])],
        options: zip::write::SimpleFileOptions,
    ) -> PathBuf {
        let path = root.join("fixture.zip");
        let mut writer = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        for (name, content) in files {
            writer
                .start_file(
                    *name,
                    options
                        .compression_method(zip::CompressionMethod::Stored)
                        .unix_permissions(0o6755),
                )
                .unwrap();
            writer.write_all(content).unwrap();
        }
        writer.finish().unwrap();
        // ZipWriterは特殊ビットを先に落とすため、悪意ある配布物の属性を中央ヘッダへ設定する。
        let mut bytes = std::fs::read(&path).unwrap();
        let headers: Vec<_> = bytes
            .windows(4)
            .enumerate()
            .filter_map(|(index, value)| (value == b"PK\x01\x02").then_some(index))
            .collect();
        for index in headers {
            bytes[index + 5] = 3;
            bytes[index + 38..index + 42].copy_from_slice(&(0o106755_u32 << 16).to_le_bytes());
        }
        std::fs::write(&path, bytes).unwrap();
        path
    }

    fn zip64_end(bytes: &[u8], extension: usize) -> Vec<u8> {
        let at = bytes.windows(4).rposition(|v| v == b"PK\x05\x06").unwrap();
        let eocd = &bytes[at..at + 22];
        let count = u64::from(u16::from_le_bytes(zip_number(eocd, 10)));
        let size = u64::from(u32::from_le_bytes(zip_number(eocd, 12)));
        let offset = u64::from(u32::from_le_bytes(zip_number(eocd, 16)));
        let mut result = bytes[..at].to_vec();
        result.extend(b"PK\x06\x06");
        result.extend((44 + extension as u64).to_le_bytes());
        result.extend([45, 0, 45, 0]);
        result.extend([0; 8]);
        for value in [count, count, size, offset] {
            result.extend(value.to_le_bytes());
        }
        result.extend(vec![0; extension]);
        result.extend(b"PK\x06\x07");
        result.extend([0; 4]);
        result.extend((at as u64).to_le_bytes());
        result.extend(1u32.to_le_bytes());
        let mut eocd = eocd.to_vec();
        eocd[8..20].fill(255);
        result.extend(eocd);
        result
    }

    fn index_reader(
        bytes: Vec<u8>,
        checking: Arc<AtomicBool>,
        rejected: Arc<AtomicBool>,
    ) -> ZipIndexReader<std::io::Cursor<Vec<u8>>> {
        ZipIndexReader {
            inner: std::io::Cursor::new(bytes),
            checking,
            limits: LIMITS,
            position: 0,
            last_read: None,
            candidate_read: false,
            rejected,
        }
    }

    #[test]
    fn 中央詳細の破損後に再探索した前方eocdとzip64の巨大索引を確保しない() {
        let temp = super::super::fs::test_directory();
        let archive = zip_file(temp.path(), &[("model", b"original")]);
        let normal = std::fs::read(archive).unwrap();
        let end = normal.windows(4).rposition(|v| v == b"PK\x05\x06").unwrap();
        let start = zip_directory(&mut std::io::Cursor::new(&normal), LIMITS)
            .unwrap()
            .start as usize;
        for zip64 in [false, true] {
            let mut prefix = if zip64 {
                zip64_end(&normal, 0)
            } else {
                normal.clone()
            };
            if zip64 {
                for offset in [24, 32] {
                    prefix[end + offset..end + offset + 8].copy_from_slice(&60000u64.to_le_bytes());
                }
            } else {
                for offset in [8, 10] {
                    prefix[end + offset..end + offset + 2].copy_from_slice(&60000u16.to_le_bytes());
                }
            }
            let mut latest = normal.clone();
            // AESを名乗るがAES extraが無い。生一覧は正常で、zipの詳細解析だけが失敗する。
            latest[start + 10..start + 12].copy_from_slice(&99u16.to_le_bytes());
            latest[end + 16..end + 20]
                .copy_from_slice(&((start + prefix.len()) as u32).to_le_bytes());
            prefix.extend(latest);
            zip_directory(&mut std::io::Cursor::new(&prefix), LIMITS).unwrap();
            let rejected = Arc::new(AtomicBool::new(false));
            let reader = index_reader(prefix, Arc::new(AtomicBool::new(true)), rejected.clone());
            assert!(zip::ZipArchive::new(reader).is_err());
            assert!(
                rejected.load(Ordering::Relaxed),
                "前方終端の確保前検査が実行されていません: zip64={zip64}"
            );
        }
    }

    #[test]
    fn 固定長の任意データは連続読込と索引gate解除後に誤拒否しない() {
        let mut bytes = b"PK\x05\x06".to_vec();
        bytes.extend([0; 18]);
        bytes[10..12].copy_from_slice(&60000u16.to_le_bytes());
        let checking = Arc::new(AtomicBool::new(true));
        let rejected = Arc::new(AtomicBool::new(false));
        let mut reader = index_reader(bytes.clone(), checking.clone(), rejected.clone());
        let mut output = [0; 22];
        reader.read_exact(&mut output).unwrap();
        assert_eq!(output.as_slice(), bytes);
        assert!(!rejected.load(Ordering::Relaxed));
        checking.store(false, Ordering::Relaxed);
        reader.seek(SeekFrom::Start(0)).unwrap();
        reader.read_exact(&mut output).unwrap();
        assert_eq!(output.as_slice(), bytes);
        assert!(!rejected.load(Ordering::Relaxed));
    }

    #[test]
    fn 索引生成前に宣言と実項目と一覧レコード境界を拒否する() {
        let temp = super::super::fs::test_directory();
        let archive = zip_file(temp.path(), &[("model", b"original")]);
        let normal = std::fs::read(archive).unwrap();
        let at = normal.windows(4).rposition(|v| v == b"PK\x05\x06").unwrap();
        let directory = zip_directory(&mut std::io::Cursor::new(&normal), LIMITS).unwrap();
        for (count, expected) in [(60000u16, "上限"), (0, "項目数が一致")] {
            let mut invalid = normal.clone();
            invalid[at + 8..at + 10].copy_from_slice(&count.to_le_bytes());
            invalid[at + 10..at + 12].copy_from_slice(&count.to_le_bytes());
            let error = zip_directory(&mut std::io::Cursor::new(invalid), LIMITS)
                .unwrap_err()
                .to_string();
            assert!(error.contains(expected), "{error}");
        }
        let mut invalid = normal.clone();
        invalid[directory.start as usize + 28..directory.start as usize + 30].fill(255);
        assert!(zip_directory(&mut std::io::Cursor::new(invalid), LIMITS).is_err());
        let mut invalid = normal[..at + 21].to_vec();
        assert!(zip_directory(&mut std::io::Cursor::new(&invalid), LIMITS).is_err());
        invalid = zip64_end(&normal, 0);
        for offset in [24, 32] {
            invalid[at + offset..at + offset + 8].copy_from_slice(&u64::MAX.to_le_bytes());
        }
        let error = zip_directory(&mut std::io::Cursor::new(invalid), LIMITS)
            .unwrap_err()
            .to_string();
        assert!(error.contains("上限"), "{error}");
        assert!(zip_directory(
            &mut std::io::Cursor::new(&normal),
            Limits {
                max_entries: 0,
                ..LIMITS
            }
        )
        .is_err());
    }

    #[test]
    fn 終端検査は空zipとコメント内の偽署名とzip64拡張とsfxを保持する() {
        let mut empty = b"PK\x05\x06".to_vec();
        empty.extend([0; 16]);
        let comment = b"comment-PK\x05\x06";
        empty.extend((comment.len() as u16).to_le_bytes());
        empty.extend(comment);
        let directory = zip_directory(&mut std::io::Cursor::new(&empty), LIMITS).unwrap();
        assert_eq!((directory.start, directory.end, directory.count), (0, 0, 0));
        let temp = super::super::fs::test_directory();
        let archive = zip_file(temp.path(), &[("model", b"original")]);
        let normal = std::fs::read(&archive).unwrap();
        for extension in [0, 70000] {
            let zipped = zip64_end(&normal, extension);
            for prefix in [0, 70000] {
                let mut bytes = vec![b'X'; prefix];
                bytes.extend(&zipped);
                std::fs::write(&archive, &bytes).unwrap();
                let directory = zip_directory(&mut std::io::Cursor::new(&bytes), LIMITS).unwrap();
                assert_eq!(directory.count, 1);
                let output = temp.path().join(format!("out-{extension}-{prefix}"));
                extract_zip(&archive, &output, LIMITS, OPTIONS).unwrap();
                assert_eq!(std::fs::read(output.join("model")).unwrap(), b"original");
            }
        }
    }

    fn tar_file(root: &Path, files: &[(&str, &[u8])], kind: tar::EntryType) -> PathBuf {
        let path = root.join("fixture.tar.gz");
        let mut writer = tar::Builder::new(Vec::new());
        for (name, content) in files {
            let mut header = tar::Header::new_gnu();
            header.set_mode(0o6755);
            header.set_size(content.len() as u64);
            header.set_entry_type(kind);
            if kind.is_symlink() || kind.is_hard_link() {
                header.set_link_name("../outside").unwrap();
            }
            header.set_cksum();
            writer.append_data(&mut header, *name, *content).unwrap();
        }
        let mut encoder = flate2::write::GzEncoder::new(
            std::fs::File::create(&path).unwrap(),
            flate2::Compression::fast(),
        );
        encoder.write_all(&writer.into_inner().unwrap()).unwrap();
        encoder.finish().unwrap();
        path
    }

    #[test]
    fn 同じ用途上限をzipとtarに適用し通常ファイルを展開する() {
        for tar in [false, true] {
            let temp = super::super::fs::test_directory();
            let files: &[(&str, &[u8])] = &[("model/file.txt", b"original")];
            let archive = if tar {
                tar_file(temp.path(), files, tar::EntryType::Regular)
            } else {
                zip_file(temp.path(), files)
            };
            let output = temp.path().join("out");
            if tar {
                extract_tar_gz(&archive, &output, LIMITS, OPTIONS).unwrap();
            } else {
                extract_zip(&archive, &output, LIMITS, OPTIONS).unwrap();
            }
            assert_eq!(
                std::fs::read(output.join("model/file.txt")).unwrap(),
                b"original"
            );
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                assert_eq!(
                    std::fs::metadata(output.join("model/file.txt"))
                        .unwrap()
                        .permissions()
                        .mode()
                        & 0o7777,
                    0o755
                );
            }
        }
    }

    #[test]
    fn 項目数と単体合計上限を展開前に拒否する() {
        for tar in [false, true] {
            for limits in [
                Limits {
                    max_entries: 0,
                    ..LIMITS
                },
                Limits {
                    max_total_bytes: 1,
                    ..LIMITS
                },
                Limits {
                    max_file_bytes: 1,
                    ..LIMITS
                },
            ] {
                let temp = super::super::fs::test_directory();
                let files: &[(&str, &[u8])] = &[("model.txt", b"original")];
                let archive = if tar {
                    tar_file(temp.path(), files, tar::EntryType::Regular)
                } else {
                    zip_file(temp.path(), files)
                };
                let output = temp.path().join("out");
                let result = if tar {
                    extract_tar_gz(&archive, &output, limits, OPTIONS)
                } else {
                    extract_zip(&archive, &output, limits, OPTIONS)
                };
                assert!(result.is_err());
                assert!(!output.exists());
            }
        }
    }

    #[test]
    fn 用途所有の拡張子上限は共通既定を置換する() {
        fn file_limit(path: &Path) -> u64 {
            if path.extension() == Some(std::ffi::OsStr::new("tga")) {
                16
            } else {
                1
            }
        }
        let temp = super::super::fs::test_directory();
        let archive = zip_file(temp.path(), &[("texture.tga", b"original")]);
        extract_zip(
            &archive,
            &temp.path().join("ok"),
            Limits {
                max_file_bytes: 1,
                file_limit: Some(file_limit),
                ..LIMITS
            },
            OPTIONS,
        )
        .unwrap();
        let archive = zip_file(temp.path(), &[("texture.png", b"original")]);
        assert!(extract_zip(
            &archive,
            &temp.path().join("bad"),
            Limits {
                file_limit: Some(file_limit),
                ..LIMITS
            },
            OPTIONS
        )
        .is_err());
    }

    #[test]
    fn zipの脱出と正規化重複を既存配置へ触れず拒否する() {
        for names in [
            ["../outside", "safe"],
            ["a", "./a"],
            ["a/b", "a\\b"],
            ["a/b", "a"],
            ["C:/outside", "safe"],
        ] {
            let temp = super::super::fs::test_directory();
            let archive = zip_file(temp.path(), &[(names[0], b"one"), (names[1], b"two")]);
            let output = temp.path().join("out");
            assert!(
                extract_zip(&archive, &output, LIMITS, OPTIONS).is_err(),
                "{} / {}",
                names[0],
                names[1]
            );
            assert!(!output.exists());
            assert!(!temp.path().join("outside").exists());
        }
    }

    #[test]
    fn tarの重複も展開前に拒否し壊れた圧縮本文を確定しない() {
        let temp = super::super::fs::test_directory();
        let archive = tar_file(
            temp.path(),
            &[("same", b"one"), ("same", b"two")],
            tar::EntryType::Regular,
        );
        let output = temp.path().join("out");
        assert!(extract_tar_gz(&archive, &output, LIMITS, OPTIONS).is_err());
        assert!(!output.exists());
        let archive = tar_file(
            temp.path(),
            &[("model", b"original")],
            tar::EntryType::Regular,
        );
        let mut bytes = std::fs::read(&archive).unwrap();
        bytes.truncate(bytes.len() - 5);
        std::fs::write(&archive, bytes).unwrap();
        assert!(extract_tar_gz(&archive, &output, LIMITS, OPTIONS).is_err());
        assert!(!output.exists());
    }

    #[cfg(unix)]
    #[test]
    fn 展開先の親リンクを拒否して外部ファイルへ触れない() {
        let temp = super::super::fs::test_directory();
        let output = temp.path().join("out");
        let external = temp.path().join("external");
        std::fs::create_dir(&output).unwrap();
        std::fs::create_dir(&external).unwrap();
        std::os::unix::fs::symlink(&external, output.join("link")).unwrap();
        let archive = zip_file(temp.path(), &[("link/model", b"original")]);
        assert!(extract_zip(
            &archive,
            &output,
            LIMITS,
            ExtractOptions {
                mode: Mode::Merge,
                ..OPTIONS
            }
        )
        .is_err());
        assert!(!external.join("model").exists());
    }

    #[test]
    fn 新規モードは旧配置を消さずmergeは同内容だけを許可する() {
        let temp = super::super::fs::test_directory();
        let output = temp.path().join("out");
        std::fs::create_dir(&output).unwrap();
        std::fs::write(output.join("existing"), b"old").unwrap();
        let archive = zip_file(temp.path(), &[("existing", b"old")]);
        assert!(extract_zip(&archive, &output, LIMITS, OPTIONS).is_err());
        let options = ExtractOptions {
            mode: Mode::Merge,
            ..OPTIONS
        };
        extract_zip(&archive, &output, LIMITS, options).unwrap();
        let archive = zip_file(temp.path(), &[("runtime.dll", b"runtime")]);
        extract_zip(&archive, &output, LIMITS, options).unwrap();
        let archive = zip_file(temp.path(), &[("existing", b"new")]);
        assert!(extract_zip(&archive, &output, LIMITS, options).is_err());
        assert_eq!(std::fs::read(output.join("existing")).unwrap(), b"old");
        assert_eq!(
            std::fs::read(output.join("runtime.dll")).unwrap(),
            b"runtime"
        );
    }

    #[test]
    fn zipとtarのリンク項目を拒否する() {
        let temp = super::super::fs::test_directory();
        let archive = temp.path().join("link.zip");
        let mut writer = zip::ZipWriter::new(std::fs::File::create(&archive).unwrap());
        writer
            .add_symlink(
                "link",
                "../outside",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        writer.finish().unwrap();
        assert!(extract_zip(&archive, &temp.path().join("zip"), LIMITS, OPTIONS).is_err());
        for kind in [
            tar::EntryType::Symlink,
            tar::EntryType::Link,
            tar::EntryType::Fifo,
        ] {
            let archive = tar_file(temp.path(), &[("link", b"")], kind);
            assert!(extract_tar_gz(&archive, &temp.path().join("tar"), LIMITS, OPTIONS).is_err());
        }
    }

    #[test]
    fn cp932の日本語名を明示した用途だけが読める() {
        let temp = super::super::fs::test_directory();
        let archive = zip_file(temp.path(), &[("aa.txt", b"original")]);
        let mut bytes = std::fs::read(&archive).unwrap();
        let positions: Vec<_> = bytes
            .windows(6)
            .enumerate()
            .filter_map(|(index, part)| (part == b"aa.txt").then_some(index))
            .collect();
        assert_eq!(positions.len(), 2);
        for index in positions {
            bytes[index..index + 2].copy_from_slice(&[0x82, 0xa0]);
        }
        std::fs::write(&archive, bytes).unwrap();
        assert!(extract_zip(&archive, &temp.path().join("utf8"), LIMITS, OPTIONS).is_err());
        extract_zip(
            &archive,
            &temp.path().join("cp932"),
            LIMITS,
            ExtractOptions {
                name_encoding: NameEncoding::Utf8OrCp932,
                ..OPTIONS
            },
        )
        .unwrap();
        assert_eq!(
            std::fs::read(temp.path().join("cp932/あ.txt")).unwrap(),
            b"original"
        );
        assert!(decode_name(&[0x81], NameEncoding::Utf8OrCp932).is_err());
    }

    #[test]
    fn 内容読込失敗は既存mergeファイルを壊さない() {
        let temp = super::super::fs::test_directory();
        let archive = zip_file(temp.path(), &[("model.txt", b"original")]);
        let mut bytes = std::fs::read(&archive).unwrap();
        let index = bytes
            .windows(8)
            .position(|value| value == b"original")
            .unwrap();
        bytes[index] = b'X';
        std::fs::write(&archive, bytes).unwrap();
        let output = temp.path().join("out");
        std::fs::create_dir(&output).unwrap();
        std::fs::write(output.join("model.txt"), b"original").unwrap();
        assert!(extract_zip(
            &archive,
            &output,
            LIMITS,
            ExtractOptions {
                mode: Mode::Merge,
                ..OPTIONS
            }
        )
        .is_err());
        assert_eq!(
            std::fs::read(output.join("model.txt")).unwrap(),
            b"original"
        );
        assert_eq!(std::fs::read_dir(output).unwrap().count(), 1);
    }

    #[test]
    fn 同一zipの生名前重複と外部一覧の同じ制約を守る() {
        let temp = super::super::fs::test_directory();
        let archive = zip_file(temp.path(), &[("aa", b"one"), ("bb", b"two")]);
        let mut bytes = std::fs::read(&archive).unwrap();
        let offsets: Vec<_> = bytes
            .windows(2)
            .enumerate()
            .filter_map(|(index, value)| (value == b"bb").then_some(index))
            .collect();
        assert_eq!(offsets.len(), 2);
        for index in offsets {
            bytes[index..index + 2].copy_from_slice(b"aa");
        }
        std::fs::write(&archive, bytes).unwrap();
        let output = temp.path().join("out");
        let error = extract_zip(&archive, &output, LIMITS, OPTIONS)
            .unwrap_err()
            .to_string();
        assert!(error.contains("同じ生名前の重複"), "{error}");
        assert!(!output.exists());
        let error = extract_zip(
            &archive,
            &output,
            Limits {
                max_entries: 1,
                ..LIMITS
            },
            OPTIONS,
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("項目が用途の上限"), "{error}");
        assert!(!output.exists());
        validate_external_entries(
            &[("./".into(), true, 0), ("model".into(), false, 8)],
            LIMITS,
        )
        .unwrap();
        for entries in [
            vec![("./".into(), true, 1)],
            vec![("../outside".into(), false, 1)],
            vec![("a".into(), false, 1), ("./a".into(), false, 1)],
            vec![("huge".into(), false, 17)],
        ] {
            assert!(validate_external_entries(&entries, LIMITS).is_err());
        }
    }

    #[test]
    fn 中央一覧の生検査はzip64サイズと書庫前置データを維持する() {
        for prefix in [false, true] {
            let temp = super::super::fs::test_directory();
            let archive = zip_file_options(
                temp.path(),
                &[("model", b"original")],
                zip::write::SimpleFileOptions::default().large_file(true),
            );
            if prefix {
                let mut bytes = b"archive-prefix".to_vec();
                bytes.extend(std::fs::read(&archive).unwrap());
                std::fs::write(&archive, bytes).unwrap();
            }
            let output = temp.path().join("out");
            extract_zip(&archive, &output, LIMITS, OPTIONS).unwrap();
            assert_eq!(std::fs::read(output.join("model")).unwrap(), b"original");
        }
    }

    #[cfg(unix)]
    #[test]
    fn 追加キャラ用の展開は書庫が実行属性を持っても付与しない() {
        use std::os::unix::fs::PermissionsExt;
        let temp = super::super::fs::test_directory();
        let archive = zip_file(temp.path(), &[("model", b"original")]);
        let mut zip = zip::ZipArchive::new(std::fs::File::open(&archive).unwrap()).unwrap();
        assert_eq!(
            zip.by_index(0).unwrap().unix_mode().unwrap() & 0o7777,
            0o6755
        );
        let output = temp.path().join("out");
        extract_zip(
            &archive,
            &output,
            LIMITS,
            ExtractOptions {
                permissions: Permissions::Plain,
                ..OPTIONS
            },
        )
        .unwrap();
        assert_eq!(
            std::fs::metadata(output.join("model"))
                .unwrap()
                .permissions()
                .mode()
                & 0o7777,
            0o644
        );
    }
}
