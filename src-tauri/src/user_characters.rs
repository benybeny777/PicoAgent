// user_characters.rs - ユーザーが自分で追加したキャラクター（Live2D モデル等）の管理。
//
// 同梱キャラクターは `ui-dist/assets/characters/` としてビルド時に実行ファイルへ焼き込まれる。
// 実行時には書き換えられないため、**追加したキャラは `app_data_dir/characters/<id>/` に置く**。
// webview へは asset プロトコル（`convertFileSrc`）で読ませる。
//
// 導入はフォルダでも zip でも受け付ける。zip は「一時フォルダへ展開 → 検証 → rename で確定」
// の順にして、検証を通らなかったものが導入先に残らないようにする。
// 検証規則は `character_pack` が正本（CLI と同じ規則を使う）。

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::Serialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

use crate::character_pack;

/// zip の展開上限（ZIP 爆弾で保存領域を埋めないための安全弁）。
const MAX_ENTRIES: usize = character_pack::MAX_FILES;
const MAX_TOTAL_BYTES: u64 = character_pack::MAX_TOTAL_BYTES;

#[derive(Serialize)]
pub struct UserCharacter {
    pub id: String,
    pub name: String,
    pub description: String,
    /// webview から読ませるための実パス（フロントで `convertFileSrc` に通す）。
    pub path: String,
    /// Live2D モデルを持つか（一覧の表示用）。
    pub live2d: bool,
}

/// 追加キャラクターの置き場（`app_data_dir/characters`）。
pub fn root(app: &AppHandle) -> PathBuf {
    app.path()
        .app_data_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("characters")
}

/// 導入済みの追加キャラクター一覧。壊れているものは飛ばして続ける。
pub fn list(app: &AppHandle) -> Vec<UserCharacter> {
    let root = root(app);
    let Ok(entries) = std::fs::read_dir(&root) else {
        // フォールバック許可: 置き場が無い（1つも追加していない）状態は正常なので空で返す。
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(path.join("character.json")) else {
            continue;
        };
        let Ok(manifest) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        let Some(id) = manifest.get("id").and_then(Value::as_str) else {
            continue;
        };
        // フォルダ名と id がずれていると読み込み先を特定できないので出さない。
        if path.file_name().and_then(|name| name.to_str()) != Some(id) {
            continue;
        }
        out.push(UserCharacter {
            id: id.to_string(),
            name: manifest
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or(id)
                .to_string(),
            description: manifest
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            path: path.to_string_lossy().to_string(),
            live2d: manifest.pointer("/live2d/model").is_some(),
        });
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

/// `character.json` が直下に無い zip（1階層余分に包まれている）を吸収する。
///
/// 配布されている Live2D パックは「zip を開くとモデル名のフォルダが1つある」形が多く、
/// そのたびにユーザーへ包み直しを強いるのは不親切なので、1階層だけ潜って探す。
fn pack_root(extracted: &Path) -> Result<PathBuf> {
    if extracted.join("character.json").is_file() {
        return Ok(extracted.to_path_buf());
    }
    let mut candidates: Vec<PathBuf> = std::fs::read_dir(extracted)?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir() && path.join("character.json").is_file())
        .collect();
    match candidates.len() {
        1 => Ok(candidates.remove(0)),
        0 => bail!("character.json が見つかりません。キャラクターパックの形式を確認してください。"),
        _ => bail!("character.json を持つフォルダが複数あります。1体分だけを選んでください。"),
    }
}

/// zip を展開先へ取り出す。パス脱出・件数・総量を展開しながら検査する。
fn extract_zip(archive: &Path, destination: &Path) -> Result<()> {
    let file = std::fs::File::open(archive)
        .with_context(|| format!("zip を開けません: {}", archive.display()))?;
    let mut zip = zip::ZipArchive::new(file).context("zip を読めません")?;
    if zip.len() > MAX_ENTRIES {
        bail!("zip の項目が多すぎます（{MAX_ENTRIES} 件まで）");
    }
    let mut total = 0u64;
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index)?;
        // `enclosed_name` は `..` や絶対パスを含むエントリに None を返す（zip slip 対策）。
        let Some(relative) = entry.enclosed_name().map(Path::to_path_buf) else {
            bail!("zip に安全でないパスが含まれています: {}", entry.name());
        };
        let target = destination.join(&relative);
        if entry.is_dir() {
            std::fs::create_dir_all(&target)?;
            continue;
        }
        total = total.saturating_add(entry.size());
        if total > MAX_TOTAL_BYTES || entry.size() > character_pack::MAX_FILE_BYTES {
            bail!("zip の中身が大きすぎます（合計250MB・1ファイル40MBまで）");
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut out = std::fs::File::create(&target)
            .with_context(|| format!("展開できません: {}", target.display()))?;
        std::io::copy(&mut entry, &mut out)?;
    }
    Ok(())
}

fn copy_dir(source: &Path, destination: &Path) -> Result<()> {
    std::fs::create_dir_all(destination)?;
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let from = entry.path();
        // シンボリックリンクは辿らない（実体がパック外を指す可能性がある）。
        if std::fs::symlink_metadata(&from)?.file_type().is_symlink() {
            bail!(
                "パック内にシンボリックリンクは置けません: {}",
                from.display()
            );
        }
        let to = destination.join(entry.file_name());
        if from.is_dir() {
            copy_dir(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

/// 配布zip／フォルダから、組み込み用の `.model3.json` を再帰的に探す。
///
/// Cubismの配布物は編集用データと `runtime/` が一緒に包まれていることが多い。
/// そのためパック直下だけでなく中を探すが、複数モデルを曖昧に選ぶことはしない。
fn find_live2d_model(root: &Path) -> Result<PathBuf> {
    fn collect(current: &Path, found: &mut Vec<PathBuf>) -> Result<()> {
        for entry in std::fs::read_dir(current)? {
            let entry = entry?;
            let path = entry.path();
            if std::fs::symlink_metadata(&path)?.file_type().is_symlink() {
                bail!(
                    "Live2Dデータ内にシンボリックリンクは置けません: {}",
                    path.display()
                );
            }
            if path.is_dir() {
                collect(&path, found)?;
            } else if path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.to_ascii_lowercase().ends_with(".model3.json"))
            {
                found.push(path);
            }
        }
        Ok(())
    }

    let mut found = Vec::new();
    collect(root, &mut found)?;
    match found.len() {
        1 => Ok(found.remove(0)),
        0 => bail!(".model3.json が見つかりません。組み込み用ファイルを含むzip／フォルダを選んでください。"),
        count => bail!(
            ".model3.json が{count}個見つかりました。1体分のruntimeフォルダ、または1体だけを含むzipを選んでください。"
        ),
    }
}

fn prepare_source(source: &Path, destination: &Path) -> Result<()> {
    if source.is_dir() {
        copy_dir(source, destination)
    } else {
        let extension = source
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if extension != "zip" {
            bail!("フォルダか .zip を指定してください: {}", source.display());
        }
        extract_zip(source, destination)
    }
}

fn cleanup_staging(app: &AppHandle, staging: &Path) {
    if staging.exists() {
        if let Err(error) = std::fs::remove_dir_all(staging) {
            crate::log_paths::append_app_log(
                app,
                &format!(
                    "キャラクター導入の一時フォルダを掃除できません: {} ({error})",
                    staging.display()
                ),
            );
        }
    }
}

fn build_live2d_pack(extracted: &Path, pack: &Path, id: &str, name: &str) -> Result<()> {
    let model = find_live2d_model(extracted)?;
    let model_root = model
        .parent()
        .context("Live2Dモデルのフォルダを特定できません")?;
    let model_name = model
        .file_name()
        .and_then(|value| value.to_str())
        .context("Live2Dのmodel3.jsonファイル名を読み取れません")?;

    copy_dir(model_root, &pack.join("live2d"))?;
    let manifest = json!({
        "id": id,
        "name": name,
        "description": "自分で追加したLive2Dモデル。",
        "greeting": format!("こんにちは、{name}です。よろしくね。"),
        "persona": {
            "personality": "親しみやすく、利用者の依頼を丁寧に手伝う。",
            "speechStyle": "自然で簡潔な日本語で話す。",
            "firstPerson": name,
            "userName": "あなた",
            "systemPrompt": format!("{name}として振る舞い、利用者の依頼を丁寧に手伝う。")
        },
        "live2d": {
            "model": format!("live2d/{model_name}"),
            "scale": 1.8
        }
    });
    crate::store::write_json_atomic(&pack.join("character.json"), &manifest)?;
    character_pack::validate(pack)?;
    Ok(())
}

/// フォルダまたは zip を導入する。導入したキャラクターの id と名前を返す。
pub fn install(app: &AppHandle, source: &Path) -> Result<(String, String)> {
    let _state_lock = crate::store::lock_state("user-characters");
    if !source.exists() {
        bail!("指定先が見つかりません: {}", source.display());
    }
    let root = root(app);
    std::fs::create_dir_all(&root)
        .with_context(|| format!("置き場を作れません: {}", root.display()))?;

    // 一時フォルダで組み立て、検証を通ってから rename で確定する
    // （途中で失敗した中途半端なフォルダを導入先へ残さない）。
    let staging = root.join(format!(".installing-{}", std::process::id()));
    if staging.exists() {
        std::fs::remove_dir_all(&staging).with_context(|| {
            format!("古い導入途中データを掃除できません: {}", staging.display())
        })?;
    }
    std::fs::create_dir_all(&staging)?;

    let result = (|| -> Result<(String, String)> {
        prepare_source(source, &staging.join("pack"))?;
        let pack = pack_root(&staging.join("pack"))?;
        let manifest = character_pack::validate(&pack)?;
        let id = character_pack::required_text(&manifest, "id")?.to_string();
        let name = character_pack::required_text(&manifest, "name")?.to_string();
        let destination = root.join(&id);
        if destination.exists() {
            bail!("同じidのキャラクターが既にあります: {id}（先に削除してください）");
        }
        std::fs::rename(&pack, &destination)
            .with_context(|| format!("導入先へ移動できません: {}", destination.display()))?;
        Ok((id, name))
    })();

    cleanup_staging(app, &staging);
    result
}

/// 利用者が自分で取得したLive2Dのzip／フォルダをPicoAgentパックへ変換して導入する。
///
/// 元データをPicoAgentが代理取得・再配布せず、利用者が使用条件を確認して入手した
/// 手元のデータだけを扱う。編集用ファイルは含めず、`.model3.json` と同じフォルダに
/// ある組み込み用データだけをユーザー領域へコピーする。
pub fn install_live2d(
    app: &AppHandle,
    source: &Path,
    id: &str,
    name: &str,
) -> Result<(String, String)> {
    let _state_lock = crate::store::lock_state("user-characters");
    if !source.exists() {
        bail!("指定先が見つかりません: {}", source.display());
    }
    let id = id.trim();
    let name = name.trim();
    if id.is_empty()
        || id.len() > character_pack::MAX_ID_CHARS
        || !id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        bail!(
            "id は{}文字以内の英数字・ハイフン・アンダースコアだけにしてください",
            character_pack::MAX_ID_CHARS
        );
    }
    if name.is_empty() {
        bail!("表示名を入力してください");
    }
    let root = root(app);
    std::fs::create_dir_all(&root)
        .with_context(|| format!("置き場を作れません: {}", root.display()))?;
    let destination = root.join(id);
    if destination.exists() {
        bail!("同じidのキャラクターが既にあります: {id}（先に削除してください）");
    }

    let staging = root.join(format!(".installing-live2d-{}", std::process::id()));
    if staging.exists() {
        std::fs::remove_dir_all(&staging).with_context(|| {
            format!("古い導入途中データを掃除できません: {}", staging.display())
        })?;
    }
    std::fs::create_dir_all(&staging)?;

    let result = (|| -> Result<(String, String)> {
        let extracted = staging.join("source");
        prepare_source(source, &extracted)?;
        let pack = staging.join("generated");
        build_live2d_pack(&extracted, &pack, id, name)?;
        let validated = character_pack::validate(&pack)?;
        let validated_id = character_pack::required_text(&validated, "id")?.to_string();
        let validated_name = character_pack::required_text(&validated, "name")?.to_string();
        std::fs::rename(&pack, &destination)
            .with_context(|| format!("導入先へ移動できません: {}", destination.display()))?;
        Ok((validated_id, validated_name))
    })();

    cleanup_staging(app, &staging);
    result
}

/// 追加キャラクターを削除する。ユーザーが持ち込んだデータなので OS のゴミ箱へ移す。
pub fn remove(app: &AppHandle, id: &str) -> Result<()> {
    let _state_lock = crate::store::lock_state("user-characters");
    if id.is_empty()
        || !id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        bail!("不正なidです: {id}");
    }
    let target = root(app).join(id);
    if !target.is_dir() {
        bail!("追加キャラクターが見つかりません: {id}");
    }
    crate::tools::trash_file::move_to_trash(&target)
        .map_err(|error| anyhow::anyhow!("ゴミ箱へ移動できません: {error}"))
}

/// 検証規則が CLI 側へ再実装されていないことを固定する設計ガード。
///
/// 以前は `xtask/src/character_pack.rs` が同じ規則のコピーを持っていた。片方だけを直すと
/// 「CLI では通るのにアプリが弾く」ズレが生まれるため、**規則の正本は
/// `src-tauri/src/character_pack.rs` 1本**にして、CLI はそれを `#[path]` で取り込む。
/// この判定は共通モジュール側に置けない（`crate::` を参照できないため）ので、
/// 取り込む側でない**アプリ側**のこのファイルに置く。
#[cfg(test)]
mod guard {
    #[test]
    fn xtaskは検証規則を自前実装しない() {
        let path = crate::guard_scan::repo_root().join("xtask/src/character_pack.rs");
        let text = std::fs::read_to_string(&path).expect("xtask の character_pack.rs を読めない");
        assert!(
            text.contains("src-tauri/src/character_pack.rs"),
            "xtask は検証規則の正本（src-tauri/src/character_pack.rs）を取り込んでください"
        );
        for marker in [
            "fn validate_tree(",
            "ALLOWED_EXTENSIONS",
            "fn safe_relative(",
        ] {
            assert!(
                !text.contains(marker),
                "xtask に検証規則のコピーが残っています（`{marker}`）。正本だけを使ってください"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{build_live2d_pack, extract_zip, find_live2d_model, pack_root};
    use std::io::Write;
    use std::path::PathBuf;

    fn temp_dir(label: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("picoagent-uc-{label}-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// 「zip を開くとフォルダが1つ」という一般的な配布形を吸収できること。
    #[test]
    fn 一階層包まれたパックも根を見つける() {
        let dir = temp_dir("root");
        std::fs::create_dir_all(dir.join("MyModel")).unwrap();
        std::fs::write(dir.join("MyModel/character.json"), b"{}").unwrap();
        assert_eq!(pack_root(&dir).unwrap(), dir.join("MyModel"));

        // 直下にある場合はそのまま。
        let flat = temp_dir("flat");
        std::fs::write(flat.join("character.json"), b"{}").unwrap();
        assert_eq!(pack_root(&flat).unwrap(), flat);

        // 複数あると 1 体に決められないので拒否する。
        let many = temp_dir("many");
        for name in ["a", "b"] {
            std::fs::create_dir_all(many.join(name)).unwrap();
            std::fs::write(many.join(name).join("character.json"), b"{}").unwrap();
        }
        assert!(pack_root(&many).unwrap_err().to_string().contains("複数"));

        for path in [dir, flat, many] {
            std::fs::remove_dir_all(path).ok();
        }
    }

    /// zip slip（`../` で展開先の外へ書き出す攻撃）を拒否すること。
    #[test]
    fn zipの危険なパスを展開しない() {
        let dir = temp_dir("zipslip");
        let archive = dir.join("evil.zip");
        {
            let file = std::fs::File::create(&archive).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let options = zip::write::FileOptions::default();
            zip.start_file("../escaped.txt", options).unwrap();
            zip.write_all(b"x").unwrap();
            zip.finish().unwrap();
        }
        let out = dir.join("out");
        let error = extract_zip(&archive, &out).unwrap_err().to_string();
        assert!(error.contains("安全でないパス"), "error: {error}");
        assert!(
            !dir.join("escaped.txt").exists(),
            "展開先の外へ書き出している"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn 通常のzipは展開できる() {
        let dir = temp_dir("zipok");
        let archive = dir.join("pack.zip");
        {
            let file = std::fs::File::create(&archive).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let options = zip::write::FileOptions::default();
            zip.start_file("MyModel/character.json", options).unwrap();
            zip.write_all(b"{}").unwrap();
            zip.finish().unwrap();
        }
        let out = dir.join("out");
        extract_zip(&archive, &out).unwrap();
        assert!(out.join("MyModel/character.json").is_file());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn 一体分のlive2dモデルを入れ子から見つける() {
        let dir = temp_dir("live2d-one");
        let runtime = dir.join("download/Character/runtime");
        std::fs::create_dir_all(&runtime).unwrap();
        std::fs::write(runtime.join("Model.model3.json"), b"{}").unwrap();
        assert_eq!(
            find_live2d_model(&dir).unwrap(),
            runtime.join("Model.model3.json")
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn 複数live2dモデルは曖昧に選ばない() {
        let dir = temp_dir("live2d-many");
        for name in ["a", "b"] {
            std::fs::create_dir_all(dir.join(name)).unwrap();
            std::fs::write(dir.join(name).join(format!("{name}.model3.json")), b"{}").unwrap();
        }
        assert!(find_live2d_model(&dir)
            .unwrap_err()
            .to_string()
            .contains("2個"));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn 生のlive2dから検証済みパックを作る() {
        let dir = temp_dir("live2d-pack");
        let source = dir.join("download/runtime");
        std::fs::create_dir_all(source.join("textures")).unwrap();
        std::fs::write(source.join("Model.moc3"), [0u8; 8]).unwrap();
        std::fs::write(source.join("textures/texture_00.png"), [0u8; 8]).unwrap();
        std::fs::write(
            source.join("Model.model3.json"),
            br#"{
              "Version": 3,
              "FileReferences": {
                "Moc": "Model.moc3",
                "Textures": ["textures/texture_00.png"]
              }
            }"#,
        )
        .unwrap();
        let pack = dir.join("pack");
        build_live2d_pack(&dir.join("download"), &pack, "my-live2d", "マイキャラ").unwrap();
        let manifest = crate::character_pack::validate(&pack).unwrap();
        assert_eq!(manifest["id"], "my-live2d");
        assert_eq!(manifest["live2d"]["model"], "live2d/Model.model3.json");
        assert_eq!(manifest["live2d"]["scale"], 1.8);
        std::fs::remove_dir_all(dir).ok();
    }
}
