// 未加工PMXの構造・計数・外部参照を導入前に検証する。アプリとxtaskで共有する。
use anyhow::{bail, Context, Result};
use serde_json::Value;
use std::path::Path;

pub(super) fn validate(root: &Path, manifest: &Value) -> Result<()> {
    let Some(mmd) = manifest.get("mmd") else {
        return Ok(());
    };
    if mmd.get("schemaVersion").and_then(Value::as_u64) != Some(1) {
        bail!("mmd.schemaVersion は1だけに対応しています");
    }
    let relative = mmd
        .get("model")
        .and_then(Value::as_str)
        .context("mmd.modelが必要です")?;
    super::ensure_asset(root, relative)?;
    let model = root.join(super::safe_relative(relative, "mmd.model")?);
    if !model
        .extension()
        .and_then(|s| s.to_str())
        .is_some_and(|s| s.eq_ignore_ascii_case("pmx"))
    {
        bail!("MMDモデルは.pmxを指定してください");
    }
    let bytes = std::fs::read(&model)?;
    let textures = inspect(&bytes)?;
    let parent = model.parent().context("PMXの親フォルダがありません")?;
    let mut unique = std::collections::BTreeSet::new();
    let mut total_pixels = 0u64;
    for texture in textures {
        if texture.is_empty() || !unique.insert(texture.clone()) {
            continue;
        }
        let normalized = texture.replace('\\', "/");
        if normalized.contains(':') || normalized.starts_with('/') {
            bail!("PMXのテクスチャに外部パスは指定できません: {texture}");
        }
        let relative = super::safe_relative(&normalized, "PMXテクスチャ")?;
        let path = parent.join(relative);
        if !path.is_file() {
            bail!("PMXテクスチャがありません: {texture}");
        }
        if !path.canonicalize()?.starts_with(root.canonicalize()?) {
            bail!("PMXテクスチャがパック外を参照しています");
        }
        let (width, height) = image_dimensions(&std::fs::read(&path)?)
            .with_context(|| format!("PMXテクスチャ寸法を読めません: {texture}"))?;
        total_pixels += u64::from(width) * u64::from(height);
        if width == 0 || height == 0 || width > 8192 || height > 8192 || total_pixels > 128_000_000
        {
            bail!("PMXテクスチャは各辺8192px・RGBA換算合計512MB以内にしてください");
        }
        let extension = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if !["png", "jpg", "jpeg", "bmp", "tga", "sph", "spa"].contains(&extension.as_str()) {
            bail!("未対応のPMXテクスチャ形式です: {texture}");
        }
    }
    Ok(())
}

// 展開前の寸法を確認し、画像デコードによるメモリ急増を防ぐ。
fn image_dimensions(bytes: &[u8]) -> Result<(u32, u32)> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") && bytes.len() >= 24 {
        return Ok((
            u32::from_be_bytes(bytes[16..20].try_into()?),
            u32::from_be_bytes(bytes[20..24].try_into()?),
        ));
    }
    if bytes.starts_with(b"BM") && bytes.len() >= 26 {
        let dib = u32::from_le_bytes(bytes[14..18].try_into()?);
        if dib == 12 {
            return Ok((
                u16::from_le_bytes(bytes[18..20].try_into()?) as u32,
                u16::from_le_bytes(bytes[20..22].try_into()?) as u32,
            ));
        }
        return Ok((
            i32::from_le_bytes(bytes[18..22].try_into()?).unsigned_abs(),
            i32::from_le_bytes(bytes[22..26].try_into()?).unsigned_abs(),
        ));
    }
    if bytes.starts_with(&[0xff, 0xd8]) {
        let mut at = 2;
        while at + 4 <= bytes.len() {
            if bytes[at] != 0xff {
                break;
            }
            let marker = bytes[at + 1];
            at += 2;
            if marker == 0xff {
                at -= 1;
                continue;
            }
            if marker == 0xd9 || marker == 0xda {
                break;
            }
            let size = u16::from_be_bytes(bytes[at..at + 2].try_into()?) as usize;
            if size < 2 || at + size > bytes.len() {
                break;
            }
            if [
                0xc0, 0xc1, 0xc2, 0xc3, 0xc5, 0xc6, 0xc7, 0xc9, 0xca, 0xcb, 0xcd, 0xce, 0xcf,
            ]
            .contains(&marker)
                && size >= 7
            {
                return Ok((
                    u16::from_be_bytes(bytes[at + 5..at + 7].try_into()?) as u32,
                    u16::from_be_bytes(bytes[at + 3..at + 5].try_into()?) as u32,
                ));
            }
            at += size;
        }
    }
    // TGAには固定のmagicがないので画像タイプと画素深度も確認する。
    if bytes.len() >= 18
        && [1, 2, 3, 9, 10, 11].contains(&bytes[2])
        && [8, 16, 24, 32].contains(&bytes[16])
    {
        return Ok((
            u16::from_le_bytes(bytes[12..14].try_into()?) as u32,
            u16::from_le_bytes(bytes[14..16].try_into()?) as u32,
        ));
    }
    bail!("対応画像のヘッダーがありません")
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
    utf8: bool,
}
impl Reader<'_> {
    fn take(&mut self, size: usize) -> Result<&[u8]> {
        let end = self.at.checked_add(size).context("PMXの長さが不正です")?;
        let bytes = self
            .bytes
            .get(self.at..end)
            .context("PMXが途中で切れています")?;
        self.at = end;
        Ok(bytes)
    }
    fn skip(&mut self, size: usize) -> Result<()> {
        self.take(size)?;
        Ok(())
    }
    fn byte(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn flags(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into()?))
    }
    fn integer(&mut self) -> Result<i32> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into()?))
    }
    fn count(&mut self, max: usize) -> Result<usize> {
        let value = self.integer()?;
        if value < 0 || value as usize > max {
            bail!("PMXの要素数が上限{max}を超えるか不正です: {value}");
        }
        Ok(value as usize)
    }
    fn text(&mut self) -> Result<String> {
        let size = self.count(1_000_000)?;
        let utf8 = self.utf8;
        let bytes = self.take(size)?;
        if utf8 {
            Ok(std::str::from_utf8(bytes)
                .context("PMXのUTF-8が不正です")?
                .to_string())
        } else {
            if bytes.len() % 2 != 0 {
                bail!("PMXのUTF-16長が不正です");
            }
            String::from_utf16(
                &bytes
                    .chunks_exact(2)
                    .map(|p| u16::from_le_bytes([p[0], p[1]]))
                    .collect::<Vec<_>>(),
            )
            .context("PMXのUTF-16が不正です")
        }
    }
    fn names(&mut self) -> Result<()> {
        self.text()?;
        self.text()?;
        Ok(())
    }
}

fn inspect(bytes: &[u8]) -> Result<Vec<String>> {
    let mut r = Reader {
        bytes,
        at: 0,
        utf8: false,
    };
    if r.take(4)? != b"PMX " {
        bail!("PMXのヘッダーが不正です");
    }
    if f32::from_le_bytes(r.take(4)?.try_into()?) != 2.0 {
        bail!("初版はPMX 2.0だけに対応しています");
    }
    if r.byte()? != 8 {
        bail!("PMXのヘッダー長が不正です");
    }
    r.utf8 = match r.byte()? {
        0 => false,
        1 => true,
        _ => bail!("PMX文字コードが不正です"),
    };
    let uv = r.byte()? as usize;
    if uv > 4 {
        bail!("PMX追加UV数が不正です");
    }
    let mut idx = [0usize; 6];
    for size in &mut idx {
        *size = r.byte()? as usize;
        if ![1, 2, 4].contains(size) {
            bail!("PMXインデックス幅が不正です");
        }
    }
    let [vertex, texture, material, bone, morph, rigid] = idx;
    r.names()?;
    r.names()?;
    let vertices = r.count(300_000)?;
    for _ in 0..vertices {
        r.skip(32 + uv * 16)?;
        match r.byte()? {
            0 => r.skip(bone)?,
            1 => r.skip(bone * 2 + 4)?,
            2 => r.skip(bone * 4 + 16)?,
            3 => r.skip(bone * 2 + 40)?,
            _ => bail!("PMX 2.0の未対応ウェイト形式です"),
        }
        r.skip(4)?;
    }
    let indices = r.count(900_000)?;
    if indices % 3 != 0 {
        bail!("PMXの面インデックス数が不正です");
    }
    for _ in 0..indices {
        let data = r.take(vertex)?;
        let index = match vertex {
            1 => data[0] as u32,
            2 => u16::from_le_bytes(data.try_into()?) as u32,
            _ => u32::from_le_bytes(data.try_into()?),
        };
        if index as usize >= vertices {
            bail!("PMXの面が存在しない頂点を参照しています");
        }
    }
    let texture_count = r.count(2_048)?;
    let mut textures = Vec::with_capacity(texture_count);
    for _ in 0..texture_count {
        textures.push(r.text()?);
    }
    let materials = r.count(2_048)?;
    let mut material_indices = 0usize;
    for _ in 0..materials {
        r.names()?;
        r.skip(65 + texture * 2)?;
        r.byte()?;
        match r.byte()? {
            0 => r.skip(texture)?,
            1 => r.skip(1)?,
            _ => bail!("PMXの共有トゥーン指定が不正です"),
        }
        r.text()?;
        material_indices += r.count(indices)?;
    }
    if material_indices != indices {
        bail!("PMXの材質と面数が一致しません");
    }
    let bones = r.count(4_096)?;
    for _ in 0..bones {
        r.names()?;
        r.skip(12 + bone + 4)?;
        let flags = r.flags()?;
        r.skip(if flags & 1 != 0 { bone } else { 12 })?;
        if flags & 0x300 != 0 {
            r.skip(bone + 4)?;
        }
        if flags & 0x400 != 0 {
            r.skip(12)?;
        }
        if flags & 0x800 != 0 {
            r.skip(24)?;
        }
        if flags & 0x2000 != 0 {
            r.skip(4)?;
        }
        if flags & 0x20 != 0 {
            r.skip(bone)?;
            r.count(1_024)?;
            r.skip(4)?;
            let links = r.count(4_096)?;
            for _ in 0..links {
                r.skip(bone)?;
                match r.byte()? {
                    0 => {}
                    1 => r.skip(24)?,
                    _ => bail!("PMXのIK制限指定が不正です"),
                }
            }
        }
    }
    let morphs = r.count(4_096)?;
    let mut offsets = 0usize;
    for _ in 0..morphs {
        r.names()?;
        r.byte()?;
        let kind = r.byte()?;
        let count = r.count(1_000_000)?;
        offsets += count;
        if offsets > 2_000_000 {
            bail!("PMXのモーフ要素が多すぎます");
        }
        let size = match kind {
            0 => morph + 4,
            1 => vertex + 12,
            2 => bone + 28,
            3..=7 => vertex + 16,
            8 => material + 113,
            _ => bail!("PMX 2.0の未対応モーフ形式です"),
        };
        r.skip(count * size)?;
    }
    let frames = r.count(4_096)?;
    for _ in 0..frames {
        r.names()?;
        r.byte()?;
        let count = r.count(8_192)?;
        for _ in 0..count {
            match r.byte()? {
                0 => r.skip(bone)?,
                1 => r.skip(morph)?,
                _ => bail!("PMX表示枠の形式が不正です"),
            }
        }
    }
    let bodies = r.count(2_048)?;
    for _ in 0..bodies {
        r.names()?;
        r.skip(bone + 61)?;
    }
    let joints = r.count(4_096)?;
    for _ in 0..joints {
        r.names()?;
        if r.byte()? != 0 {
            bail!("PMX 2.0の未対応ジョイントです");
        }
        r.skip(rigid * 2 + 96)?;
    }
    if r.at != bytes.len() {
        bail!("PMX 2.0の末尾に未対応データがあります");
    }
    Ok(textures)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn empty() -> Vec<u8> {
        let mut bytes = b"PMX ".to_vec();
        bytes.extend(2.0f32.to_le_bytes());
        bytes.extend([8, 1, 0, 1, 1, 1, 1, 1, 1]);
        bytes.extend([0u8; 52]);
        bytes
    }
    #[test]
    fn 空モデルの構造と破損を区別する() {
        let bytes = empty();
        assert!(inspect(&bytes).is_ok());
        for len in [0, 4, 8, 17, bytes.len() - 1] {
            assert!(inspect(&bytes[..len]).is_err());
        }
    }
    #[test]
    fn 過大な頂点数を読み込む前に拒否する() {
        let mut bytes = empty();
        bytes[33..37].copy_from_slice(&300_001i32.to_le_bytes());
        assert!(inspect(&bytes).unwrap_err().to_string().contains("上限"));
    }
    #[test]
    fn テクスチャの欠損と外部参照を拒否する() {
        let root = std::env::temp_dir().join(format!("pico-mmd-test-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let manifest = serde_json::json!({"mmd":{"schemaVersion":1,"model":"test.pmx"}});
        for (texture, expected) in [
            ("missing.png", "ありません"),
            ("../escape.png", ".."),
            ("C:/escape.png", "外部パス"),
        ] {
            let mut bytes = empty();
            bytes[41..45].copy_from_slice(&1i32.to_le_bytes());
            let mut text = (texture.len() as i32).to_le_bytes().to_vec();
            text.extend(texture.as_bytes());
            bytes.splice(45..45, text);
            std::fs::write(root.join("test.pmx"), bytes).unwrap();
            assert!(validate(&root, &manifest)
                .unwrap_err()
                .to_string()
                .contains(expected));
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn 非圧縮画像の寸法を取得する() {
        let mut bytes = [0u8; 18];
        bytes[2] = 2;
        bytes[16] = 32;
        bytes[12..14].copy_from_slice(&4096u16.to_le_bytes());
        bytes[14..16].copy_from_slice(&4096u16.to_le_bytes());
        assert_eq!(image_dimensions(&bytes).unwrap(), (4096, 4096));
        assert!(image_dimensions(&[0; 18]).is_err());
    }

    #[test]
    fn 新版を黙って旧版として解釈しない() {
        let mut bytes = empty();
        bytes[4..8].copy_from_slice(&2.1f32.to_le_bytes());
        assert!(inspect(&bytes).is_err());
    }
}
