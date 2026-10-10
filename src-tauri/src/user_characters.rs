// user_characters.rs - ユーザーが自分で追加したキャラクター（Live2D モデル等）の管理。
//
// 同梱キャラクターは `ui-dist/assets/characters/` としてビルド時に実行ファイルへ焼き込まれる。
// 実行時には書き換えられないため、**追加したキャラは `PicoAgent/characters/<id>/` に置く**。
// webview へは asset プロトコル（`convertFileSrc`）で読ませる。
//
// 導入はフォルダでも zip でも受け付ける。zip は「一時フォルダへ展開 → 検証 → rename で確定」
// の順にして、検証を通らなかったものが導入先に残らないようにする。
// 検証規則は `character_pack` が正本（CLI と同じ規則を使う）。

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::Serialize;
use serde_json::{json, Value};
use tauri::AppHandle;

use crate::character_pack;
use crate::installer::{
    archive::{extract_zip, ExtractOptions, Mode, NameEncoding, Permissions},
    fs::copy_dir_plain as copy_dir,
    Limits,
};

const PACK_LIMITS: Limits = Limits {
    max_entries: character_pack::MAX_FILES,
    max_total_bytes: character_pack::MAX_TOTAL_BYTES,
    max_file_bytes: character_pack::MAX_FILE_BYTES,
    file_limit: Some(character_pack::file_limit),
};
const PACK_OPTIONS: ExtractOptions = ExtractOptions {
    mode: Mode::Fresh,
    name_encoding: NameEncoding::Utf8OrCp932,
    permissions: Permissions::Plain,
};

/// zip の展開上限（ZIP 爆弾で保存領域を埋めないための安全弁）。
const MAX_TOTAL_BYTES: u64 = character_pack::MAX_TOTAL_BYTES;
const MAX_NAME_CHARS: usize = 80;

#[derive(Serialize)]
pub struct UserCharacter {
    pub id: String,
    pub name: String,
    pub description: String,
    /// webview から読ませるための実パス（フロントで `convertFileSrc` に通す）。
    pub path: String,
    /// Live2D モデルを持つか（一覧の表示用）。
    pub live2d: bool,
    pub mmd: bool,
}

/// 追加キャラクターの置き場（`PicoAgent/characters`）。
pub fn root(app: &AppHandle) -> Result<PathBuf> {
    Ok(crate::app_paths::data_path(
        app,
        crate::app_paths::DataPath::Characters,
    ))
}

/// 導入済みの追加キャラクター一覧。壊れているものは飛ばして続ける。
pub fn list(app: &AppHandle) -> Result<Vec<UserCharacter>> {
    let _state_lock = crate::store::lock_state("user-characters");
    let root = root(app)?;
    log_recovery(app, recover_interrupted_work(&root));
    let entries = match std::fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("追加キャラクターを列挙できません: {}", root.display()))
        }
    };
    let mut out = Vec::new();
    for entry in entries {
        let entry = entry.context("追加キャラクターの項目を読み取れません")?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let manifest_path = path.join("character.json");
        let text = match std::fs::read_to_string(&manifest_path) {
            Ok(text) => text,
            Err(error) => {
                crate::log_paths::append_app_log(
                    app,
                    &format!(
                        "追加キャラクターの設定を読めないため一覧から除外します: {} ({error})",
                        manifest_path.display()
                    ),
                );
                continue;
            }
        };
        let manifest = match serde_json::from_str::<Value>(&text) {
            Ok(manifest) => manifest,
            Err(error) => {
                crate::log_paths::append_app_log(
                    app,
                    &format!(
                        "追加キャラクターの設定が壊れているため一覧から除外します: {} ({error})",
                        manifest_path.display()
                    ),
                );
                continue;
            }
        };
        let Some(id) = manifest.get("id").and_then(Value::as_str) else {
            crate::log_paths::append_app_log(
                app,
                &format!(
                    "追加キャラクターのidが無いため一覧から除外します: {}",
                    manifest_path.display()
                ),
            );
            continue;
        };
        // フォルダ名と id がずれていると読み込み先を特定できないので出さない。
        if path.file_name().and_then(|name| name.to_str()) != Some(id) {
            crate::log_paths::append_app_log(
                app,
                &format!(
                    "追加キャラクターのフォルダ名とidが一致しないため一覧から除外します: {} (id={id})",
                    path.display()
                ),
            );
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
            mmd: manifest.pointer("/mmd/model").is_some(),
        });
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(out)
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
        copy_dir(source, destination, PACK_LIMITS)
    } else {
        let extension = source
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if extension != "zip" {
            bail!("フォルダか .zip を指定してください: {}", source.display());
        }
        extract_zip(source, destination, PACK_LIMITS, PACK_OPTIONS)
    }
}

/// 異常終了で残った導入・更新の途中データを片付ける（呼び出し側が `user-characters` のロックを持つ）。
///
/// 途中データの名前はプロセスIDを含むため、再起動後の新しいプロセスは同じ名前を見ない。
/// ここで拾わないと、更新中に落ちたキャラは退避先 `.update-backup-*` に残ったまま一覧から消え、
/// 導入途中の `.installing-*` / `.updating-*` も容量を占めたまま溜まり続ける。
/// - 退避先は、元の場所が空いていれば戻し（旧版の復元）、新版が既にあれば消す。
/// - 導入途中のフォルダは消す（検証前の中途半端な内容で、確定済みのものは無い）。
///
/// 返り値はログへ残す文。失敗しても一覧・導入は止めない（次の機会にやり直す）。
fn recover_interrupted_work(root: &Path) -> Vec<String> {
    let mut messages = Vec::new();
    let Ok(entries) = std::fs::read_dir(root) else {
        return messages;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !path.is_dir() {
            continue;
        }
        if name.starts_with(".update-backup-") {
            let id = std::fs::read(path.join("character.json"))
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
                .and_then(|manifest| manifest.get("id")?.as_str().map(str::to_string))
                .filter(|id| character_pack::is_valid_id(id));
            let Some(id) = id else {
                messages.push(format!(
                    "中断した更新の退避先からキャラクターIDを読めないため残します: {}",
                    path.display()
                ));
                continue;
            };
            let destination = root.join(&id);
            if destination.exists() {
                match std::fs::remove_dir_all(&path) {
                    Ok(()) => {
                        messages.push(format!("更新済みキャラクターの退避先を消しました: {id}"))
                    }
                    Err(error) => messages.push(format!(
                        "更新済みキャラクターの退避先を消せません: {} ({error})",
                        path.display()
                    )),
                }
            } else {
                match std::fs::rename(&path, &destination) {
                    Ok(()) => messages.push(format!(
                        "中断した更新から元のキャラクターを戻しました: {id}"
                    )),
                    Err(error) => messages.push(format!(
                        "中断した更新から元のキャラクターを戻せません: {} ({error})",
                        path.display()
                    )),
                }
            }
        } else if name.starts_with(".installing-") || name.starts_with(".updating-") {
            if let Err(error) = std::fs::remove_dir_all(&path) {
                messages.push(format!(
                    "導入途中のフォルダを消せません: {} ({error})",
                    path.display()
                ));
            } else {
                messages.push(format!(
                    "導入途中のフォルダを消しました: {}",
                    path.display()
                ));
            }
        }
    }
    messages
}

fn log_recovery(app: &AppHandle, messages: Vec<String>) {
    for message in messages {
        crate::log_paths::append_app_log(app, &message);
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

    copy_dir(model_root, &pack.join("live2d"), PACK_LIMITS)?;
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
            // モデル固有の顔・胸位置は model3.json から共通には判定できない。
            // まず全身が枠へ収まる安全な倍率で導入し、利用者の配置調整で寄せる。
            "scale": 1.0,
            "offsetY": 0
        }
    });
    crate::store::write_json_atomic(&pack.join("character.json"), &manifest)?;
    character_pack::validate(pack)?;
    Ok(())
}

fn build_mmd_pack(
    extracted: &Path,
    selected: Option<&Path>,
    pack: &Path,
    id: &str,
    name: &str,
) -> Result<()> {
    fn collect(root: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
        for entry in std::fs::read_dir(root)? {
            let path = entry?.path();
            if path.is_dir() {
                collect(&path, out)?;
            } else if path
                .extension()
                .and_then(|s| s.to_str())
                .is_some_and(|s| s.eq_ignore_ascii_case("pmx"))
            {
                out.push(path);
            }
        }
        Ok(())
    }
    let model = if let Some(selected) = selected {
        selected.to_path_buf()
    } else {
        let mut models = Vec::new();
        collect(extracted, &mut models)?;
        if models.len() != 1 {
            bail!(
                "PMXが{}個あります。使用する1体の.pmxファイルを指定してください",
                models.len()
            );
        }
        models.remove(0)
    };
    let relative = model
        .strip_prefix(extracted)?
        .to_string_lossy()
        .replace('\\', "/");
    copy_dir(extracted, &pack.join("mmd"), PACK_LIMITS)?;
    let manifest = json!({
        "id": id, "name": name, "description": "自分で追加したMMDモデル。",
        "greeting": format!("こんにちは、{name}です。よろしくね。"),
        "persona": { "personality": "親しみやすく、利用者の依頼を丁寧に手伝う。", "speechStyle": "自然で簡潔な日本語で話す。", "firstPerson": name, "userName": "あなた", "systemPrompt": format!("{name}として振る舞い、利用者の依頼を丁寧に手伝う。") },
        "mmd": { "schemaVersion": 1, "model": format!("mmd/{relative}") }
    });
    crate::store::write_json_atomic(&pack.join("character.json"), &manifest)?;
    character_pack::validate(pack)?;
    Ok(())
}

/// PMXを直接選んだ場合、ダウンロードフォルダ全体ではなく参照素材だけを取り込む。
fn copy_selected_pmx(source: &Path, destination: &Path) -> Result<PathBuf> {
    let parent = source.parent().context("PMXの親フォルダがありません")?;
    let source_root = parent.canonicalize()?;
    let name = source.file_name().context("PMX名がありません")?;
    if !source.canonicalize()?.starts_with(&source_root) {
        bail!("PMXが指定フォルダ外を参照しています");
    }
    if std::fs::metadata(source)?.len() > character_pack::file_limit(source) {
        bail!("PMXが大きすぎます");
    }
    let bytes = std::fs::read(source)?;
    let textures = character_pack::mmd_texture_references(&bytes)?;
    std::fs::create_dir_all(destination)?;
    let selected = destination.join(name);
    crate::store::write_atomic(&selected, &bytes)?;
    let mut total = bytes.len() as u64;
    let mut copied = std::collections::HashSet::new();
    for texture in textures {
        if texture.is_empty() || !copied.insert(texture.clone()) {
            continue;
        }
        let normalized = texture.replace('\\', "/");
        if normalized.contains(':') || normalized.starts_with('/') {
            bail!("PMXのテクスチャに外部パスは指定できません: {texture}");
        }
        let relative = character_pack::safe_relative(&normalized, "PMXテクスチャ")?;
        let from = parent.join(&relative);
        // 取り込み後の検証と同じ文言で、欠けたテクスチャ名を利用者へ示す。
        if !from.is_file() {
            bail!("PMXテクスチャがありません: {texture}");
        }
        if !from.canonicalize()?.starts_with(&source_root) {
            bail!("PMXテクスチャがモデルフォルダ外を参照しています: {texture}");
        }
        let size = std::fs::metadata(&from)?.len();
        total = total.saturating_add(size);
        if size > character_pack::file_limit(&from) || total > MAX_TOTAL_BYTES {
            bail!("PMXとテクスチャの合計が導入上限を超えています");
        }
        let to = destination.join(relative);
        if let Some(folder) = to.parent() {
            std::fs::create_dir_all(folder)?;
        }
        std::fs::copy(&from, to)?;
    }
    Ok(selected)
}

/// フォルダまたは zip を導入する。導入したキャラクターの id と名前を返す。
/// 配信物の一覧から同梱キャラクターのidを取り出す（`characters` 配列の `id`）。
fn parse_bundled_ids(index_json: &str) -> Vec<String> {
    let Ok(index) = serde_json::from_str::<Value>(index_json) else {
        return Vec::new();
    };
    index
        .get("characters")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.get("id").and_then(Value::as_str))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// 同梱キャラクターと同じidのパックは、一覧で同梱側が優先されるため追加しても選べない。
/// 一覧（`assets/characters/index.json`）はビルド時に実行ファイルへ焼き込まれるので、
/// ファイルではなく配信アセットとして読む（開発時は `frontendDist` から読まれる）。
fn ensure_id_not_bundled(app: &AppHandle, id: &str) -> Result<()> {
    let Some(asset) = app
        .asset_resolver()
        .get("assets/characters/index.json".to_string())
    else {
        // 一覧を読めない構成では判定しない（設定画面側でも同じ衝突を弾いている）。
        return Ok(());
    };
    let Ok(text) = String::from_utf8(asset.bytes) else {
        return Ok(());
    };
    if parse_bundled_ids(&text).iter().any(|bundled| bundled == id) {
        bail!("同梱キャラクターと同じidです: {id}（パックのidを変えてください）");
    }
    Ok(())
}

pub fn install(app: &AppHandle, source: &Path) -> Result<(String, String)> {
    let _state_lock = crate::store::lock_state("user-characters");
    if !source.exists() {
        bail!("指定先が見つかりません: {}", source.display());
    }
    let root = root(app)?;
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
        ensure_id_not_bundled(app, &id)?;
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

/// キャラ作成機能が導入したパックだけを、同じ生成元IDの完成世代で置き換える。
/// 利用者が別経路で追加した同名IDは上書きせず、既存の設定上書きはIDを維持して残す。
pub fn install_or_update_generated(
    app: &AppHandle,
    source: &Path,
    source_character_id: &str,
) -> Result<(String, String, bool)> {
    let _state_lock = crate::store::lock_state("user-characters");
    let root = root(app)?;
    std::fs::create_dir_all(&root)?;
    log_recovery(app, recover_interrupted_work(&root));
    let staging = root.join(format!(".updating-{}", std::process::id()));
    let backup = root.join(format!(".update-backup-{}", std::process::id()));
    if staging.exists() || backup.exists() {
        bail!(
            "以前のキャラクター更新の途中データを片付けられません。ログの理由を確認してください: {}",
            root.display()
        );
    }
    copy_dir(source, &staging, PACK_LIMITS)?;
    let result = (|| -> Result<(String, String, bool)> {
        let manifest = character_pack::validate(&staging)?;
        let id = character_pack::required_text(&manifest, "id")?.to_string();
        let name = character_pack::required_text(&manifest, "name")?.to_string();
        ensure_id_not_bundled(app, &id)?;
        let generated_id = manifest
            .pointer("/characterMaker/sourceCharacterId")
            .and_then(Value::as_str);
        if generated_id != Some(source_character_id) {
            bail!("生成元IDが一致しないため導入できません: {source_character_id}");
        }
        let destination = root.join(&id);
        let updated = destination.exists();
        if updated {
            let current: Value =
                serde_json::from_slice(&std::fs::read(destination.join("character.json"))?)?;
            let current_source = current
                .pointer("/characterMaker/sourceCharacterId")
                .and_then(Value::as_str);
            if current_source != Some(source_character_id) {
                bail!("同じIDの別キャラクターが既にあります: {id}");
            }
            std::fs::rename(&destination, &backup)?;
        }
        if let Err(error) = std::fs::rename(&staging, &destination) {
            if updated {
                if let Err(restore) = std::fs::rename(&backup, &destination) {
                    // 戻せなかった旧版は退避先に残る。次に一覧を開いたとき
                    // `recover_interrupted_work` が元の場所へ戻す。
                    bail!(
                        "更新したキャラクターを配置できず、元のキャラクターも戻せませんでした（次回の起動時に戻します）: {error} / {restore}"
                    );
                }
            }
            return Err(error.into());
        }
        if updated {
            // 新版はもう配置済みなので、旧版の掃除に失敗しても更新は成功として返す
            // （退避先は次回 `recover_interrupted_work` が消す）。
            if let Err(error) = std::fs::remove_dir_all(&backup) {
                crate::log_paths::append_app_log(
                    app,
                    &format!(
                        "更新前のキャラクターを掃除できません（次回の起動時に消します）: {} ({error})",
                        backup.display()
                    ),
                );
            }
        }
        Ok((id, name, updated))
    })();
    if staging.exists() {
        cleanup_staging(app, &staging);
    }
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
    if !character_pack::is_valid_id(id) {
        bail!(
            "id は{}文字以内の英数字・ハイフン・アンダースコアだけにしてください",
            character_pack::MAX_ID_CHARS
        );
    }
    if name.is_empty() {
        bail!("表示名を入力してください");
    }
    ensure_id_not_bundled(app, id)?;
    let root = root(app)?;
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

/// 未加工のPMXと素材を利用者領域へ導入する。
pub fn install_mmd(
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
    if !character_pack::is_valid_id(id) {
        bail!(
            "id は{}文字以内の英数字・ハイフン・アンダースコアだけにしてください",
            character_pack::MAX_ID_CHARS
        );
    }
    if name.is_empty() {
        bail!("表示名を入力してください");
    }
    ensure_id_not_bundled(app, id)?;
    let root = root(app)?;
    std::fs::create_dir_all(&root)
        .with_context(|| format!("置き場を作れません: {}", root.display()))?;
    let destination = root.join(id);
    if destination.exists() {
        bail!("同じidのキャラクターが既にあります: {id}（先に削除してください）");
    }

    let staging = root.join(format!(".installing-mmd-{}", std::process::id()));
    if staging.exists() {
        std::fs::remove_dir_all(&staging).with_context(|| {
            format!("古い導入途中データを掃除できません: {}", staging.display())
        })?;
    }
    std::fs::create_dir_all(&staging)?;

    let result = (|| -> Result<(String, String)> {
        let extracted = staging.join("source");
        let selected = if source
            .extension()
            .and_then(|s| s.to_str())
            .is_some_and(|s| s.eq_ignore_ascii_case("pmx"))
        {
            Some(copy_selected_pmx(source, &extracted)?)
        } else {
            prepare_source(source, &extracted)?;
            None
        };
        let pack = staging.join("generated");
        build_mmd_pack(&extracted, selected.as_deref(), &pack, id, name)?;
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
    if !character_pack::is_valid_id(id) {
        bail!("不正なidです: {id}");
    }
    let target = root(app)?.join(id);
    if !target.is_dir() {
        bail!("追加キャラクターが見つかりません: {id}");
    }
    crate::tools::trash_file::move_to_trash(&target)
        .map_err(|error| anyhow::anyhow!("ゴミ箱へ移動できません: {error}"))
}

/// PicoAgentがLive2D導入時に生成する挨拶の型。
const GREETING_TEMPLATES: [fn(&str) -> String; 1] =
    [|base| format!("こんにちは、{base}です。よろしくね。")];

/// PicoAgentがLive2D導入時に生成する人格プロンプトの型。
const PROMPT_TEMPLATES: [fn(&str) -> String; 1] =
    [|base| format!("{base}として振る舞い、利用者の依頼を丁寧に手伝う。")];

fn rename_manifest(manifest: &mut Value, name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > MAX_NAME_CHARS {
        bail!("表示名は1〜{MAX_NAME_CHARS}文字にしてください");
    }
    let old_name = character_pack::required_text(manifest, "name")?.to_string();
    let object = manifest
        .as_object_mut()
        .context("character.json の形式が不正です")?;
    object.insert("name".to_string(), Value::String(name.to_string()));

    // PicoAgentが自動生成した定型文だけは新しい名前へ追従させる。
    // 利用者が書き換えた挨拶・人格は文字列が一致しないので上書きしない。
    let old_base = old_name.as_str();
    let new_base = name;
    if let Some(current) = object.get("greeting").and_then(Value::as_str) {
        if let Some(make) = GREETING_TEMPLATES
            .iter()
            .find(|make| make(old_base) == current)
        {
            object.insert("greeting".to_string(), Value::String(make(new_base)));
        }
    }
    if let Some(persona) = object.get_mut("persona").and_then(Value::as_object_mut) {
        // 一人称が表示名そのままの場合だけ追従させる。
        match persona.get("firstPerson").and_then(Value::as_str) {
            Some(value) if value == old_name => {
                persona.insert("firstPerson".to_string(), Value::String(name.to_string()));
            }
            Some(value) if value == old_base => {
                persona.insert(
                    "firstPerson".to_string(),
                    Value::String(new_base.to_string()),
                );
            }
            _ => {}
        }
        if let Some(current) = persona.get("systemPrompt").and_then(Value::as_str) {
            if let Some(make) = PROMPT_TEMPLATES
                .iter()
                .find(|make| make(old_base) == current)
            {
                persona.insert("systemPrompt".to_string(), Value::String(make(new_base)));
            }
        }
    }
    Ok(name.to_string())
}

/// 追加キャラクターの表示名を変更する。内部IDとフォルダ名は参照に使うため変更しない。
pub fn rename(app: &AppHandle, id: &str, name: &str) -> Result<String> {
    let _state_lock = crate::store::lock_state("user-characters");
    if !character_pack::is_valid_id(id) {
        bail!("不正なidです: {id}");
    }
    let target = root(app)?.join(id);
    if !target.is_dir() {
        bail!("追加キャラクターが見つかりません: {id}");
    }
    let mut manifest = character_pack::validate(&target)?;
    let name = rename_manifest(&mut manifest, name)?;
    crate::store::write_json_atomic(&target.join("character.json"), &manifest)?;
    Ok(name)
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
    /// `character_pack.rs` は xtask が `#[path]` で取り込むため、`crate::` の
    /// アプリ内部モジュールを参照してはいけない（依存は std / serde_json / anyhow だけ）。
    ///
    /// この制約は AGENTS.md に文章でだけ書かれていて検査が無く、実際に設計ガードを
    /// あのファイルへ足したときに `crate::guard_scan` を参照して xtask のビルドが壊れた。
    /// 壊れるのは xtask 側だけなので、アプリのテストを回しても気づけない。
    #[test]
    fn 検証規則の正本はアプリ内部へ依存しない() {
        let path = crate::guard_scan::repo_root().join("src-tauri/src/character_pack.rs");
        let text = std::fs::read_to_string(&path).expect("character_pack.rs を読めない");
        let violations: Vec<String> = text
            .lines()
            .enumerate()
            .filter(|(_, line)| !line.trim_start().starts_with("//"))
            .filter(|(_, line)| line.contains("crate::"))
            .map(|(number, line)| format!("{}: {}", number + 1, line.trim()))
            .collect();
        assert!(
            violations.is_empty(),
            "character_pack.rs は xtask からも取り込まれるため crate:: を参照できません\
             （設計ガードを足すなら user_characters.rs 側へ置いてください）:\n{}",
            violations.join("\n")
        );
    }

    /// キャラクターIDの判定を写して持たないこと。
    ///
    /// 同じ規則が9箇所へ写されており、`character_framing` / `stream_profile` /
    /// 構図保存 / `user_characters::remove` の4つでは**長さ制限が抜けていた**
    /// （文字種だけ見ていて、いくら長いIDでも通っていた）。IDはフォルダ名と設定キーの
    /// 一部になるため、判定は `character_pack::is_valid_id()` の1箇所に保つ。
    ///
    /// このガードを `character_pack.rs` へ置いてはいけない。あのファイルは xtask が
    /// `#[path]` で取り込むため `crate::` のアプリ内部（`guard_scan` 等）を参照できない。
    fn is_id_rejection(line: &str, window: &str) -> bool {
        let rejects = line.contains(".all(")
            && line.contains("is_ascii_alphanumeric()")
            && (line.contains('-') || line.contains('_'));
        if !rejects {
            return false;
        }
        // 判定対象がキャラクターIDのときだけ。ツール名・サーバ名の検査や、
        // `.filter()` / `.map()` で落とす正規化は用途が違うので含めない。
        window.contains("character_id") || window.contains("id.is_empty()")
    }

    #[test]
    fn キャラクターidの判定を自前実装しない() {
        let mut violations = Vec::new();
        for source in crate::guard_scan::all_sources() {
            if source.name == "character_pack.rs" {
                continue; // 判定の定義元
            }
            let lines: Vec<&str> = source.body().lines().collect();
            for (index, line) in lines.iter().enumerate() {
                if line.trim_start().starts_with("//") {
                    continue;
                }
                let from = index.saturating_sub(6);
                let window = lines[from..=index].join("\n");
                if is_id_rejection(line, &window) {
                    violations.push(format!("{}:{}", source.path.display(), index + 1));
                }
            }
        }
        assert!(
            violations.is_empty(),
            "キャラクターIDの判定は character_pack::is_valid_id() を使ってください\
             （写すと長さ制限が抜けます）: {violations:?}"
        );
    }

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
    use super::{
        build_live2d_pack, find_live2d_model, pack_root, parse_bundled_ids,
        recover_interrupted_work, rename_manifest,
    };
    use serde_json::json;
    use std::path::PathBuf;

    fn temp_dir(_label: &str) -> PathBuf {
        crate::installer::fs::test_directory().keep()
    }

    fn write_character(dir: &std::path::Path, id: &str, marker: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(
            dir.join("character.json"),
            json!({ "id": id, "name": marker }).to_string(),
        )
        .unwrap();
    }

    /// 更新中に落ちて退避先だけが残ったキャラは、再起動後（別のプロセスID）に元の場所へ戻る。
    #[test]
    fn 中断した更新の退避先から元のキャラを戻す() {
        let root = temp_dir("recover-restore");
        write_character(&root.join(".update-backup-111"), "mine", "旧版");

        let messages = recover_interrupted_work(&root);

        let restored = std::fs::read_to_string(root.join("mine/character.json")).unwrap();
        assert!(
            restored.contains("旧版"),
            "元のキャラが戻っていません: {restored}"
        );
        assert!(!root.join(".update-backup-111").exists());
        assert!(messages
            .iter()
            .any(|message| message.contains("戻しました")));
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// 新版の配置まで済んでいれば、退避先の旧版は消す（新版を旧版で上書きしない）。
    #[test]
    fn 更新済みなら退避先の旧版を消して新版を残す() {
        let root = temp_dir("recover-updated");
        write_character(&root.join("mine"), "mine", "新版");
        write_character(&root.join(".update-backup-222"), "mine", "旧版");

        recover_interrupted_work(&root);

        let current = std::fs::read_to_string(root.join("mine/character.json")).unwrap();
        assert!(
            current.contains("新版"),
            "新版が旧版で上書きされました: {current}"
        );
        assert!(!root.join(".update-backup-222").exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// 導入途中の一時フォルダは片付け、導入済みのキャラと読めない退避先には触らない。
    #[test]
    fn 導入途中のフォルダだけを片付けて導入済みキャラは残す() {
        let root = temp_dir("recover-staging");
        write_character(&root.join("kept"), "kept", "導入済み");
        write_character(&root.join(".installing-live2d-333"), "half", "途中");
        write_character(&root.join(".updating-444"), "half", "途中");
        std::fs::create_dir_all(root.join(".update-backup-555")).unwrap();

        recover_interrupted_work(&root);

        assert!(root.join("kept/character.json").exists());
        assert!(!root.join(".installing-live2d-333").exists());
        assert!(!root.join(".updating-444").exists());
        assert!(
            root.join(".update-backup-555").exists(),
            "IDを読めない退避先は利用者のデータかもしれないので消さない"
        );
        std::fs::remove_dir_all(&root).unwrap();
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

    #[test]
    fn 複数pmxは自動で選ばず直接指定なら原本を保持する() {
        let dir = temp_dir("mmd-select");
        let source = dir.join("source");
        std::fs::create_dir_all(&source).unwrap();
        let mut pmx = b"PMX ".to_vec();
        pmx.extend(2.0f32.to_le_bytes());
        pmx.extend([8, 1, 0, 1, 1, 1, 1, 1, 1]);
        pmx.extend([0u8; 52]);
        for name in ["a.pmx", "b.pmx"] {
            std::fs::write(source.join(name), &pmx).unwrap();
        }
        assert!(
            super::build_mmd_pack(&source, None, &dir.join("ambiguous"), "test", "テスト")
                .unwrap_err()
                .to_string()
                .contains("2個")
        );
        let pack = dir.join("pack");
        super::build_mmd_pack(
            &source,
            Some(&source.join("b.pmx")),
            &pack,
            "test",
            "テスト",
        )
        .unwrap();
        let manifest = crate::character_pack::validate(&pack).unwrap();
        assert_eq!(manifest["mmd"]["model"], "mmd/b.pmx");
        assert_eq!(std::fs::read(pack.join("mmd/b.pmx")).unwrap(), pmx);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn 直接選んだpmxは隣の無関係なダウンロードを取り込まない() {
        let dir = temp_dir("selected-pmx-only");
        std::fs::create_dir_all(&dir).unwrap();
        let mut pmx = b"PMX ".to_vec();
        pmx.extend(2.0f32.to_le_bytes());
        pmx.extend([8, 1, 0, 1, 1, 1, 1, 1, 1]);
        pmx.extend([0u8; 52]);
        let texture = "tex/skin.png";
        pmx[41..45].copy_from_slice(&1i32.to_le_bytes());
        let mut reference = (texture.len() as i32).to_le_bytes().to_vec();
        reference.extend(texture.as_bytes());
        pmx.splice(45..45, reference);
        let source = dir.join("selected.pmx");
        std::fs::write(&source, &pmx).unwrap();
        std::fs::create_dir_all(dir.join("tex")).unwrap();
        std::fs::write(dir.join(texture), b"texture").unwrap();
        std::fs::write(dir.join("unrelated.txt"), b"private").unwrap();
        let destination = dir.join("copied");
        let selected = super::copy_selected_pmx(&source, &destination).unwrap();
        assert_eq!(std::fs::read(selected).unwrap(), pmx);
        assert_eq!(
            std::fs::read(destination.join(texture)).unwrap(),
            b"texture"
        );
        assert!(!destination.join("unrelated.txt").exists());
        std::fs::remove_file(dir.join(texture)).unwrap();
        let missing = super::copy_selected_pmx(&source, &dir.join("missing"))
            .unwrap_err()
            .to_string();
        assert!(missing.contains("PMXテクスチャがありません: tex/skin.png"));
        std::fs::remove_dir_all(dir).unwrap();
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
        assert_eq!(manifest["live2d"]["scale"], 1.0);
        assert_eq!(manifest["live2d"]["offsetY"], 0);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn 配信物の一覧から同梱キャラのidだけを取り出す() {
        let ids = parse_bundled_ids(
            r#"{"default":"mugi","characters":[{"id":"mugi","name":"むぎ"},{"name":"id無し"},{"id":"rin"}]}"#,
        );
        assert_eq!(ids, vec!["mugi".to_string(), "rin".to_string()]);
        // 読めない一覧では判定材料を持たない（呼び出し側は素通しする）。
        assert!(parse_bundled_ids("{").is_empty());
        assert!(parse_bundled_ids(r#"{"characters":{}}"#).is_empty());
    }

    #[test]
    fn 追加キャラの名前変更は定型人格だけ追従する() {
        let mut manifest = json!({
            "id": "my-live2d",
            "name": "旧名",
            "greeting": "こんにちは、旧名です。よろしくね。",
            "persona": {
                "firstPerson": "旧名",
                "systemPrompt": "旧名として振る舞い、利用者の依頼を丁寧に手伝う。",
                "speechStyle": "独自の口調"
            }
        });
        assert_eq!(rename_manifest(&mut manifest, " 新名 ").unwrap(), "新名");
        assert_eq!(manifest["name"], "新名");
        assert_eq!(manifest["greeting"], "こんにちは、新名です。よろしくね。");
        assert_eq!(manifest["persona"]["firstPerson"], "新名");
        assert_eq!(manifest["persona"]["speechStyle"], "独自の口調");
    }
}
