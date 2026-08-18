// xtask/src/assets.rs - キャラ同期・UI 配信ビルド・残存プロセス停止。
//
// 旧 `scripts/sync-characters.mjs` / `build-ui-dist.mjs` / `stop-stale.mjs` の置き換え。

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use image::codecs::webp::WebPEncoder;
use image::imageops::FilterType;
use image::{ExtendedColorType, ImageReader};
use sha2::{Digest, Sha256};

use crate::util::{
    list_files, log, repo_root, sync_dir, sync_dir_with_equivalence, warn, write_atomic,
};

const DELIVERY_IMAGE_POLICY: &str = "lanczos3-webp-lossless-v1";
const DELIVERY_IMAGE_MAX_HEIGHT: u32 = 1800;

#[derive(Clone, Debug)]
struct DeliveryImage {
    character_id: String,
    source: PathBuf,
    relative: PathBuf,
    width: u32,
    height: u32,
    reusable: bool,
}

#[derive(Default)]
struct DeliveryPlan {
    by_source: HashMap<PathBuf, DeliveryImage>,
    desired_metadata: HashMap<String, Option<serde_json::Value>>,
}

/// ルートの `characters/`（正本）を webview 配信先 `ui/assets/characters/` へ同期する。
///
/// キャラは扱いやすいようプロジェクト直下を正本にしているが、webview は
/// `frontendDist`（`ui/`）配下しか配信できないため、dev / build 前にコピーして配る。
/// 生成物は `.gitignore` 対象。
pub fn sync_characters() -> Result<()> {
    sync_characters_with_force(false)
}

/// 通常同期。`force` のときだけ、検証後に配信先を全再構築する。
pub fn sync_characters_with_force(force: bool) -> Result<()> {
    let root = repo_root();
    let src = root.join("characters");
    let dest = root.join("ui/assets/characters");
    if !src.exists() {
        bail!("キャラクターの正本が見つかりません: {}", src.display());
    }
    // 配信先を消す前に判定する。消してから失敗すると、キャラが1体も居ない状態が残り、
    // 次の `dev` / `build` は `ui-dist/` を作れないままになる。
    let unfetched = unfetched_live2d_characters(&src)?;
    validate_character_index(&src)?;
    let runtime_layer_sources = validate_character_sources(&src, &|id| {
        !unfetched.iter().any(|unfetched_id| unfetched_id == id)
    })?;
    let delivery_plan = build_delivery_plan(&src, &dest, &|id| {
        !unfetched.iter().any(|unfetched_id| unfetched_id == id)
    })?;
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    // live2d-src/ と expression-patches/ は制作素材で実行時に使わないため配信へ含めない。
    // モデル未取得の Live2D サンプルも、正本直下のフォルダごと外す。
    let stats = sync_dir_with_equivalence(
        &src,
        &dest,
        &|path| {
            if is_character_source_only(path) {
                return true;
            }
            if runtime_layer_sources
                .iter()
                .any(|source_layers| path.starts_with(source_layers))
            {
                return true;
            }
            path.parent() == Some(src.as_path())
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| unfetched.iter().any(|id| id == name))
        },
        force,
        &|source, target| sync_files_equivalent(source, target, &delivery_plan),
    )?;
    if !unfetched.is_empty() {
        drop_characters_from_index(&dest, &unfetched)?;
        warn(
            "sync-characters",
            format!(
                "Live2Dサンプル{}体はモデルが手元に無いため同期から外しました（使うなら cargo xtask setup live2d-samples）: {}",
                unfetched.len(),
                unfetched.join(", ")
            ),
        );
    }
    let mut changed_characters = changed_character_ids(&dest, &stats.changed_paths);
    changed_characters.extend(apply_delivery_plan(&dest, &delivery_plan)?);
    stamp_character_asset_versions(&dest, Some(&changed_characters))?;
    log(
        "sync-characters",
        format!(
            "{} -> {} を同期しました（コピー{}件、変更なし{}件、削除{}件）",
            src.display(),
            dest.display(),
            stats.copied_files,
            stats.unchanged_files,
            stats.removed_entries
        ),
    );
    Ok(())
}

/// モデル一式が手元に無い Live2D キャラのIDを集める。
///
/// `characters/*/live2d/` は再配布条件があるため Git 管理外で、
/// `cargo xtask setup live2d-samples` を実行した環境にしか無い。一方 `character.json` は
/// Git 管理下にあり Live2D を宣言しているため、clone 直後は「宣言だけあってモデルが無い」
/// 状態になる。ここで外しておかないと同期が中断し、`ui-dist/` が作られず
/// `tauri::generate_context!` がコンパイル時に落ちて**アプリが起動しなくなる**。
///
/// 宣言したモデルが**1つも無い**ものだけを未取得と見なす。一部だけ欠けているのは
/// 取得が壊れているか書き出し漏れなので、外さずに後段の検証でエラーにする。
fn unfetched_live2d_characters(src: &Path) -> Result<Vec<String>> {
    let mut character_dirs = fs::read_dir(src)?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir() && path.join("character.json").is_file())
        .collect::<Vec<_>>();
    character_dirs.sort();

    let mut unfetched = Vec::new();
    for character_dir in character_dirs {
        let manifest_path = character_dir.join("character.json");
        let manifest: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&manifest_path)?).with_context(|| {
                format!("character.json を読めません: {}", manifest_path.display())
            })?;
        if manifest.get("live2d").is_none() {
            continue;
        }
        // 定義自体が壊れている場合は判定せず、検証側の詳しいエラーへ委ねる。
        let Ok(models) = live2d_model_paths(&manifest) else {
            continue;
        };
        if models
            .iter()
            .any(|model| character_dir.join(model).is_file())
        {
            continue;
        }
        if let Some(id) = character_dir.file_name().and_then(|name| name.to_str()) {
            unfetched.push(id.to_string());
        }
    }
    Ok(unfetched)
}

/// index と各キャラ正本を、配信先へ触る前に検証する。
fn validate_character_index(src: &Path) -> Result<()> {
    let index_path = src.join("index.json");
    let index: serde_json::Value = serde_json::from_str(&fs::read_to_string(&index_path)?)
        .with_context(|| format!("index.json を読めません: {}", index_path.display()))?;
    if index
        .get("characters")
        .and_then(serde_json::Value::as_array)
        .is_none()
    {
        bail!("{} にcharacters配列がありません", index_path.display());
    }
    Ok(())
}

/// 対象キャラを検証し、実行時コピーから外せる制作正本レイヤーの場所を返す。
fn validate_character_sources(src: &Path, include: &dyn Fn(&str) -> bool) -> Result<Vec<PathBuf>> {
    let mut character_dirs = fs::read_dir(src)?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir() && path.join("character.json").is_file())
        .collect::<Vec<_>>();
    character_dirs.sort();

    let mut runtime_layer_sources = Vec::new();
    for character_dir in character_dirs {
        let Some(id) = character_dir.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !include(id) {
            continue;
        }
        let manifest_path = character_dir.join("character.json");
        let manifest: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&manifest_path)?).with_context(|| {
                format!("character.json を読めません: {}", manifest_path.display())
            })?;
        validate_character_asset_generation(&character_dir, &manifest)?;
        if manifest
            .get("runtimeLayers")
            .and_then(serde_json::Value::as_object)
            .is_some()
        {
            runtime_layer_sources.push(character_dir.join("layers"));
        }
    }
    Ok(runtime_layer_sources)
}

/// 自動版と配信用画像情報以外が同じmanifestは、配信先の生成情報を保つ。
fn character_manifests_equivalent(source: &Path, target: &Path) -> Result<bool> {
    if source.file_name().and_then(|name| name.to_str()) != Some("character.json") {
        return Ok(false);
    }
    let Ok(mut source_manifest) =
        serde_json::from_str::<serde_json::Value>(&fs::read_to_string(source)?)
    else {
        return Ok(false);
    };
    let Ok(mut target_manifest) =
        serde_json::from_str::<serde_json::Value>(&fs::read_to_string(target)?)
    else {
        return Ok(false);
    };
    if !target_manifest
        .get("assetVersion")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|version| version.starts_with("auto-"))
    {
        return Ok(false);
    }
    for manifest in [&mut source_manifest, &mut target_manifest] {
        if let Some(object) = manifest.as_object_mut() {
            object.remove("assetVersion");
            object.remove("deliveryImages");
        }
    }
    Ok(source_manifest == target_manifest)
}

fn sync_files_equivalent(source: &Path, target: &Path, plan: &DeliveryPlan) -> Result<bool> {
    if character_manifests_equivalent(source, target)? {
        return Ok(true);
    }
    let Some(image) = plan.by_source.get(source) else {
        return Ok(false);
    };
    Ok(image.reusable && delivery_image_matches(target, image.width, image.height)?)
}

/// 高精細な制作正本から、WebViewが実際に使う寸法の配信用コピーを計画する。
/// 正本manifestには生成情報を書かず、配信先のmanifestだけへ元画像ハッシュを残す。
fn build_delivery_plan(
    src: &Path,
    dest: &Path,
    include: &dyn Fn(&str) -> bool,
) -> Result<DeliveryPlan> {
    let mut plan = DeliveryPlan::default();
    let mut character_dirs = fs::read_dir(src)?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir() && path.join("character.json").is_file())
        .collect::<Vec<_>>();
    character_dirs.sort();

    for character_dir in character_dirs {
        let Some(id) = character_dir
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_owned)
        else {
            continue;
        };
        if !include(&id) {
            continue;
        }
        plan.desired_metadata.insert(id.clone(), None);
        let manifest_path = character_dir.join("character.json");
        let manifest: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&manifest_path)?)?;
        if manifest
            .get("animationQualityMode")
            .and_then(serde_json::Value::as_str)
            != Some("sharp-expression")
        {
            continue;
        }

        let target_manifest = dest
            .join(&id)
            .join("character.json")
            .is_file()
            .then(|| fs::read_to_string(dest.join(&id).join("character.json")))
            .transpose()?
            .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok());
        let mut paths = referenced_delivery_images(&manifest)?;
        paths.sort();
        paths.dedup();
        let mut hashes = serde_json::Map::new();

        for relative in paths {
            let source = character_dir.join(&relative);
            if !source.is_file() {
                bail!("配信用画像の正本が見つかりません: {}", source.display());
            }
            let (source_width, source_height) = image_dimensions(&source)
                .with_context(|| format!("画像寸法を読めません: {}", source.display()))?;
            if source_height <= DELIVERY_IMAGE_MAX_HEIGHT {
                continue;
            }
            if source.extension().and_then(|extension| extension.to_str()) != Some("webp") {
                bail!(
                    "高精細な実行画像はlossless WebPにしてください: {}",
                    source.display()
                );
            }
            let width = ((u64::from(source_width) * u64::from(DELIVERY_IMAGE_MAX_HEIGHT)
                + u64::from(source_height) / 2)
                / u64::from(source_height)) as u32;
            let source_hash = file_sha256(&source)?;
            let key = relative.to_string_lossy().replace('\\', "/");
            hashes.insert(key.clone(), serde_json::Value::String(source_hash.clone()));
            let target = dest.join(&id).join(&relative);
            let reusable = target_manifest.as_ref().is_some_and(|existing| {
                existing.get("deliveryImages").is_some_and(|delivery| {
                    delivery.get("policy").and_then(serde_json::Value::as_str)
                        == Some(DELIVERY_IMAGE_POLICY)
                        && delivery
                            .get("maxHeight")
                            .and_then(serde_json::Value::as_u64)
                            == Some(u64::from(DELIVERY_IMAGE_MAX_HEIGHT))
                        && delivery
                            .get("sources")
                            .and_then(|sources| sources.get(&key))
                            .and_then(serde_json::Value::as_str)
                            == Some(source_hash.as_str())
                })
            }) && delivery_image_matches(&target, width, DELIVERY_IMAGE_MAX_HEIGHT)?;
            plan.by_source.insert(
                source.clone(),
                DeliveryImage {
                    character_id: id.clone(),
                    source,
                    relative,
                    width,
                    height: DELIVERY_IMAGE_MAX_HEIGHT,
                    reusable,
                },
            );
        }

        if !hashes.is_empty() {
            plan.desired_metadata.insert(
                id,
                Some(serde_json::json!({
                    "policy": DELIVERY_IMAGE_POLICY,
                    "maxHeight": DELIVERY_IMAGE_MAX_HEIGHT,
                    "sources": hashes,
                })),
            );
        }
    }
    Ok(plan)
}

fn referenced_delivery_images(manifest: &serde_json::Value) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    if let Some(layers) = manifest
        .get("runtimeLayers")
        .and_then(serde_json::Value::as_object)
    {
        for value in layers.values() {
            let relative = value
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("runtimeLayersのパスが文字列ではありません"))?;
            paths.push(safe_delivery_path(relative)?);
        }
    }
    if let Some(animations) = manifest
        .get("animations")
        .and_then(serde_json::Value::as_object)
    {
        for animation in animations.values() {
            if let Some(frame_paths) = animation
                .get("framePaths")
                .and_then(serde_json::Value::as_array)
            {
                for value in frame_paths {
                    let relative = value.as_str().ok_or_else(|| {
                        anyhow::anyhow!("animations.framePathsのパスが文字列ではありません")
                    })?;
                    paths.push(safe_delivery_path(relative)?);
                }
            }
        }
    }
    Ok(paths)
}

fn safe_delivery_path(relative: &str) -> Result<PathBuf> {
    let path = PathBuf::from(relative);
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        bail!("配信用画像は安全な相対パスにしてください: {relative}");
    }
    Ok(path)
}

fn image_dimensions(path: &Path) -> Result<(u32, u32)> {
    Ok(ImageReader::open(path)?
        .with_guessed_format()?
        .into_dimensions()?)
}

fn file_sha256(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

fn delivery_image_matches(path: &Path, width: u32, height: u32) -> Result<bool> {
    if !path.is_file() || !is_lossless_webp(path)? {
        return Ok(false);
    }
    Ok(image_dimensions(path).is_ok_and(|dimensions| dimensions == (width, height)))
}

fn write_delivery_image(source: &Path, target: &Path, width: u32, height: u32) -> Result<()> {
    let decoded = ImageReader::open(source)?
        .with_guessed_format()?
        .decode()?
        .to_rgba8();
    let resized = image::imageops::resize(&decoded, width, height, FilterType::Lanczos3);
    let mut encoded = Vec::new();
    WebPEncoder::new_lossless(&mut encoded).encode(
        resized.as_raw(),
        width,
        height,
        ExtendedColorType::Rgba8,
    )?;
    write_atomic(target, &encoded)
}

/// 計画した画像だけを配信先で縮小し、正本ハッシュを配信manifestへ記録する。
fn apply_delivery_plan(dest: &Path, plan: &DeliveryPlan) -> Result<HashSet<String>> {
    // 元画像ハッシュを先に記録する。途中中断後も、寸法と形式まで検証済みの画像だけを
    // 次回再利用でき、未生成分は正本コピーのままなので必ず再生成される。
    let mut changed = apply_delivery_metadata(dest, plan)?;
    let mut source_size = 0_u64;
    let mut images = plan.by_source.values().cloned().collect::<Vec<_>>();
    images.sort_by(|left, right| {
        (&left.character_id, &left.relative).cmp(&(&right.character_id, &right.relative))
    });

    let mut pending = Vec::new();
    for image in &images {
        let target = dest.join(&image.character_id).join(&image.relative);
        source_size += fs::metadata(&image.source)?.len();
        if !(image.reusable && delivery_image_matches(&target, image.width, image.height)?) {
            pending.push(image.clone());
        }
    }
    let converted = pending.len();
    changed.extend(convert_delivery_images(dest, pending)?);
    let target_size = images.iter().try_fold(0_u64, |total, image| {
        Ok::<_, anyhow::Error>(
            total + fs::metadata(dest.join(&image.character_id).join(&image.relative))?.len(),
        )
    })?;

    if !plan.by_source.is_empty() {
        log(
            "sync-characters",
            format!(
                "配信用画像{}件を確認しました（再生成{}件、正本{:.2}MB -> 配信{:.2}MB）",
                plan.by_source.len(),
                converted,
                source_size as f64 / 1_000_000.0,
                target_size as f64 / 1_000_000.0
            ),
        );
    }
    Ok(changed)
}

fn convert_delivery_images(dest: &Path, pending: Vec<DeliveryImage>) -> Result<HashSet<String>> {
    if pending.is_empty() {
        return Ok(HashSet::new());
    }
    let workers = std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1)
        .min(4)
        .min(pending.len());
    let mut chunks = vec![Vec::new(); workers];
    for (index, image) in pending.into_iter().enumerate() {
        chunks[index % workers].push(image);
    }
    let results = std::thread::scope(|scope| {
        chunks
            .into_iter()
            .map(|chunk| {
                scope.spawn(move || -> Result<Vec<String>> {
                    let mut changed = Vec::new();
                    for image in chunk {
                        let target = dest.join(&image.character_id).join(&image.relative);
                        write_delivery_image(&image.source, &target, image.width, image.height)?;
                        if !delivery_image_matches(&target, image.width, image.height)? {
                            bail!("配信用画像の検証に失敗しました: {}", target.display());
                        }
                        changed.push(image.character_id);
                    }
                    Ok(changed)
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|handle| handle.join())
            .collect::<Vec<_>>()
    });
    let mut changed = HashSet::new();
    for result in results {
        let ids =
            result.map_err(|_| anyhow::anyhow!("配信用画像の変換スレッドが異常終了しました"))??;
        changed.extend(ids);
    }
    Ok(changed)
}

fn apply_delivery_metadata(dest: &Path, plan: &DeliveryPlan) -> Result<HashSet<String>> {
    let mut changed = HashSet::new();
    let mut metadata = plan.desired_metadata.iter().collect::<Vec<_>>();
    metadata.sort_by(|left, right| left.0.cmp(right.0));
    for (id, desired) in metadata {
        let manifest_path = dest.join(id).join("character.json");
        if !manifest_path.is_file() {
            continue;
        }
        let mut manifest: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&manifest_path)?)?;
        let current = manifest.get("deliveryImages");
        if current == desired.as_ref() {
            continue;
        }
        let object = manifest
            .as_object_mut()
            .ok_or_else(|| anyhow::anyhow!("character.jsonのルートがobjectではありません"))?;
        match desired {
            Some(value) => {
                object.insert("deliveryImages".to_string(), value.clone());
            }
            None => {
                object.remove("deliveryImages");
            }
        }
        write_atomic(
            &manifest_path,
            format!("{}\n", serde_json::to_string_pretty(&manifest)?).as_bytes(),
        )?;
        changed.insert(id.clone());
    }
    Ok(changed)
}

fn changed_character_ids(dest: &Path, paths: &[PathBuf]) -> HashSet<String> {
    paths
        .iter()
        .filter_map(|path| match path.components().next() {
            Some(Component::Normal(id)) => id.to_str().map(str::to_owned),
            _ => None,
        })
        .filter(|id| dest.join(id).join("character.json").is_file())
        .collect()
}

/// 同期から外したキャラを、配信先の `index.json` からも取り除く。
///
/// 一覧に残したままだと、設定画面が「選べるのに読み込めないキャラ」を見せてしまう。
fn drop_characters_from_index(dest: &Path, dropped: &[String]) -> Result<()> {
    let index_path = dest.join("index.json");
    let mut index: serde_json::Value = serde_json::from_str(&fs::read_to_string(&index_path)?)?;
    let characters = index
        .get_mut("characters")
        .and_then(serde_json::Value::as_array_mut)
        .ok_or_else(|| anyhow::anyhow!("{} にcharacters配列がありません", index_path.display()))?;
    characters.retain(|character| {
        character
            .get("id")
            .and_then(serde_json::Value::as_str)
            .is_none_or(|id| !dropped.iter().any(|removed| removed == id))
    });
    let Some(first) = characters
        .first()
        .and_then(|character| character.get("id"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
    else {
        bail!(
            "同期できるキャラクターが1体もありません: {}",
            index_path.display()
        );
    };
    // 既定キャラを外した場合だけ、残っている先頭へ寄せる（キャラ不在で起動しないため）。
    if index
        .get("default")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|default| dropped.iter().any(|removed| removed == default))
    {
        index["default"] = serde_json::Value::String(first);
    }
    fs::write(
        &index_path,
        format!("{}\n", serde_json::to_string_pretty(&index)?),
    )?;
    Ok(())
}

/// 販売設定で明示したキャラだけを配信先へ同期する。
///
/// 開発用の公式サンプルや制作途中モデルを誤って販売物へ含めないため、通常同期とは
/// 別入口にする。元のcharacters/は変更せず、生成先のindex.jsonだけを絞り込む。
pub fn sync_distribution_characters(included: &[String]) -> Result<()> {
    let root = repo_root();
    let src = root.join("characters");
    let dest = root.join("ui/assets/characters");
    if !src.is_dir() {
        bail!("キャラクターの正本が見つかりません: {}", src.display());
    }
    if included.is_empty() {
        bail!("販売対象キャラが空です");
    }

    let index_path = src.join("index.json");
    let mut index: serde_json::Value = serde_json::from_str(&fs::read_to_string(&index_path)?)?;
    let characters = index
        .get_mut("characters")
        .and_then(serde_json::Value::as_array_mut)
        .ok_or_else(|| anyhow::anyhow!("characters/index.json にcharacters配列がありません"))?;
    characters.retain(|character| {
        character
            .get("id")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|id| included.iter().any(|allowed| allowed == id))
    });
    if characters.len() != included.len() {
        bail!(
            "販売設定のキャラ件数とindex.jsonの一致件数が異なります: 設定{}件 / 一致{}件",
            included.len(),
            characters.len()
        );
    }
    let default = index
        .get("default")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    if !included.iter().any(|id| id == default) {
        index["default"] = serde_json::Value::String(included[0].clone());
    }
    for id in included {
        let manifest = src.join(id).join("character.json");
        if !manifest.is_file() {
            bail!(
                "販売対象キャラの正本が見つかりません: {}",
                manifest.display()
            );
        }
    }
    let index_text = format!("{}\n", serde_json::to_string_pretty(&index)?);
    let included_set = included.iter().cloned().collect::<HashSet<_>>();
    let runtime_layer_sources = validate_character_sources(&src, &|id| included_set.contains(id))?;
    let delivery_plan = build_delivery_plan(&src, &dest, &|id| included_set.contains(id))?;
    let stats = sync_dir_with_equivalence(
        &src,
        &dest,
        &|path| {
            if path == index_path || is_character_source_only(path) {
                return true;
            }
            if runtime_layer_sources
                .iter()
                .any(|source_layers| path.starts_with(source_layers))
            {
                return true;
            }
            path.parent() == Some(src.as_path())
                && path.is_dir()
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|id| !included_set.contains(id))
        },
        false,
        &|source, target| sync_files_equivalent(source, target, &delivery_plan),
    )?;
    fs::write(dest.join("index.json"), index_text)?;
    let mut changed_characters = changed_character_ids(&dest, &stats.changed_paths);
    changed_characters.extend(apply_delivery_plan(&dest, &delivery_plan)?);
    stamp_character_asset_versions(&dest, Some(&changed_characters))?;
    log(
        "sync-characters",
        format!(
            "販売対象{}体だけを同期しました（コピー{}件、変更なし{}件、削除{}件）: {}",
            included.len(),
            stats.copied_files,
            stats.unchanged_files,
            stats.removed_entries,
            included.join(", ")
        ),
    );
    Ok(())
}

fn is_character_source_only(path: &Path) -> bool {
    path.components().any(|component| {
        component.as_os_str().to_str().is_some_and(|name| {
            matches!(
                name,
                "live2d-src" | "expression-patches" | "runtime-layers.part"
            ) || (name.starts_with("frames-") && name.ends_with(".part"))
        })
    })
}

/// 同期済みキャラの全内容から自動版を作り、manifestへ書き込む。
///
/// 手書き版だけでは画像差し替え時に更新を忘れ、WebViewが旧画像を表示し続ける。
/// 配信するファイル自体のハッシュなら、通常の同期だけでURLが変わりキャッシュが切り替わる。
fn stamp_character_asset_versions(dest: &Path, changed: Option<&HashSet<String>>) -> Result<()> {
    let mut character_dirs = fs::read_dir(dest)?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir() && path.join("character.json").is_file())
        .collect::<Vec<_>>();
    character_dirs.sort();

    for character_dir in character_dirs {
        if changed.is_some_and(|ids| {
            character_dir
                .file_name()
                .and_then(|name| name.to_str())
                .is_none_or(|id| !ids.contains(id))
        }) {
            continue;
        }
        let manifest_path = character_dir.join("character.json");
        let mut manifest: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&manifest_path)?)?;
        validate_character_runtime_generation(&character_dir, &manifest)?;
        remove_source_layers_from_runtime_copy(&character_dir, &manifest)?;
        let version = character_content_version(&character_dir)?;
        manifest["assetVersion"] = serde_json::Value::String(format!("auto-{version}"));
        fs::write(
            manifest_path,
            format!("{}\n", serde_json::to_string_pretty(&manifest)?),
        )?;
    }
    Ok(())
}

fn is_lossless_webp(path: &Path) -> Result<bool> {
    let mut bytes = Vec::with_capacity(16);
    fs::File::open(path)?.take(16).read_to_end(&mut bytes)?;
    Ok(bytes.len() == 16
        && &bytes[..4] == b"RIFF"
        && &bytes[8..12] == b"WEBP"
        && &bytes[12..16] == b"VP8L")
}

/// 旧同期で残った制作正本のlayers/*.pngを配信用コピーからだけ外す。
/// 現行同期は正本側で検証してコピーを省くが、実行時WebPの検証はここでも行う。
fn remove_source_layers_from_runtime_copy(
    character_dir: &Path,
    manifest: &serde_json::Value,
) -> Result<()> {
    let Some(runtime_layers) = manifest
        .get("runtimeLayers")
        .and_then(serde_json::Value::as_object)
    else {
        return Ok(());
    };
    if manifest
        .get("runtimeLayerEncoding")
        .and_then(serde_json::Value::as_str)
        != Some("webp-lossless-v1")
    {
        bail!("runtimeLayersを使う場合はruntimeLayerEncoding=webp-lossless-v1が必要です");
    }
    for name in FULL_MOTION_LAYERS {
        let relative = runtime_layers
            .get(*name)
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("runtimeLayers.{name} がありません"))?;
        let relative_path = Path::new(relative);
        if relative_path.is_absolute()
            || relative_path
                .components()
                .any(|component| !matches!(component, std::path::Component::Normal(_)))
        {
            bail!("runtimeLayers.{name} は安全な相対パスにしてください: {relative}");
        }
        let path = character_dir.join(relative_path);
        if path.extension().and_then(|extension| extension.to_str()) != Some("webp")
            || !path.is_file()
            || !is_lossless_webp(&path)?
        {
            bail!(
                "runtimeLayers.{name} がlossless WebPを指していません: {}",
                path.display()
            );
        }
    }
    let source_layers = character_dir.join("layers");
    if source_layers.exists() {
        fs::remove_dir_all(source_layers)?;
    }
    Ok(())
}

const THA3_MOTION_LAYERS: [&str; 12] = [
    "idle_breathe",
    "idle_left",
    "idle_right",
    "sad",
    "sad_blink",
    "talking_left_closed",
    "talking_left_open",
    "talking_right_closed",
    "talking_right_open",
    "thinking_blink",
    "thinking_left",
    "thinking_right",
];

const SHARP_EXPRESSION_MINIMUM_SIZE: (u32, u32) = (1780, 3600);

fn png_dimensions(path: &Path) -> Result<(u32, u32)> {
    let mut bytes = [0_u8; 24];
    fs::File::open(path)?.read_exact(&mut bytes)?;
    if &bytes[..8] != b"\x89PNG\r\n\x1a\n" || &bytes[12..16] != b"IHDR" {
        bail!("PNGヘッダーを読み取れません: {}", path.display());
    }
    Ok((
        u32::from_be_bytes(bytes[16..20].try_into()?),
        u32::from_be_bytes(bytes[20..24].try_into()?),
    ))
}

fn validate_sharp_expression_pack(
    character_dir: &Path,
    manifest: &serde_json::Value,
) -> Result<()> {
    if manifest
        .get("animationQualityMode")
        .and_then(serde_json::Value::as_str)
        != Some("sharp-expression")
    {
        return Ok(());
    }
    let stale = THA3_MOTION_LAYERS
        .iter()
        .filter(|name| {
            character_dir
                .join("layers")
                .join(format!("{name}.png"))
                .is_file()
        })
        .copied()
        .collect::<Vec<_>>();
    if !stale.is_empty() {
        bail!(
            "{} はsharp-expressionですが旧THA3全身レイヤーが残っています: {}",
            character_dir.display(),
            stale.join(", ")
        );
    }
    for name in FULL_MOTION_LAYERS {
        let path = character_dir.join("layers").join(format!("{name}.png"));
        let (width, height) = png_dimensions(&path)?;
        if width < SHARP_EXPRESSION_MINIMUM_SIZE.0 || height < SHARP_EXPRESSION_MINIMUM_SIZE.1 {
            bail!(
                "{} はsharp-expressionですが{}が低解像度です: {}x{}（最低{}x{}）",
                character_dir.display(),
                name,
                width,
                height,
                SHARP_EXPRESSION_MINIMUM_SIZE.0,
                SHARP_EXPRESSION_MINIMUM_SIZE.1
            );
        }
    }
    Ok(())
}

const FULL_MOTION_LAYERS: &[&str] = &[
    "base",
    "blink_half",
    "blink_closed",
    "mouth_closed",
    "mouth_open",
    "gaze_left",
    "gaze_right",
    "expression_happy",
    "expression_sad",
];

fn validate_full_motion_pack(character_dir: &Path, manifest: &serde_json::Value) -> Result<()> {
    if manifest
        .get("motionProfile")
        .and_then(serde_json::Value::as_str)
        != Some("full")
    {
        return Ok(());
    }
    let missing = FULL_MOTION_LAYERS
        .iter()
        .filter(|name| {
            !character_dir
                .join("layers")
                .join(format!("{name}.png"))
                .is_file()
        })
        .copied()
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        bail!(
            "{} はmotionProfile=fullですが共通モーションレイヤーが不足しています: {}",
            character_dir.display(),
            missing.join(", ")
        );
    }
    Ok(())
}

fn validate_character_asset_generation(
    character_dir: &Path,
    manifest: &serde_json::Value,
) -> Result<()> {
    if manifest.get("live2d").is_some() {
        return validate_character_runtime_generation(character_dir, manifest);
    }
    validate_sharp_expression_pack(character_dir, manifest)?;
    validate_full_motion_pack(character_dir, manifest)?;
    validate_character_runtime_generation(character_dir, manifest)
}

/// 配信用コピーだけで確認できるフレーム・Live2D・世代整合性を検証する。
fn validate_character_runtime_generation(
    character_dir: &Path,
    manifest: &serde_json::Value,
) -> Result<()> {
    if manifest.get("live2d").is_some() {
        return validate_live2d_model_sets(character_dir, manifest);
    }
    let generation = manifest
        .get("frameGeneration")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            anyhow::anyhow!("{} にframeGenerationがありません", character_dir.display())
        })?;
    let expected_prefix = format!("frames/{generation}/");
    let animations = manifest
        .get("animations")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| anyhow::anyhow!("{} にanimationsがありません", character_dir.display()))?;
    for (name, animation) in animations {
        let paths = animation
            .get("framePaths")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| anyhow::anyhow!("animations.{name} にframePathsがありません"))?;
        for path in paths {
            let path = path.as_str().unwrap_or("");
            if !path.starts_with(&expected_prefix) {
                bail!(
                    "{} の{}が現世代{}以外を参照しています: {}",
                    character_dir.display(),
                    name,
                    generation,
                    path
                );
            }
            let asset = character_dir.join(path);
            if !asset.is_file() {
                bail!(
                    "{} の{}が存在しません: {}",
                    character_dir.display(),
                    name,
                    path
                );
            }
            if manifest
                .get("frameEncoding")
                .and_then(serde_json::Value::as_str)
                == Some("webp-lossless-v1")
                && (Path::new(path)
                    .extension()
                    .and_then(|extension| extension.to_str())
                    != Some("webp")
                    || !is_lossless_webp(&asset)?)
            {
                bail!(
                    "{} の{}がlossless WebPではありません: {}",
                    character_dir.display(),
                    name,
                    path
                );
            }
        }
    }
    let frame_root = character_dir.join("frames");
    let entries = fs::read_dir(&frame_root)?
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    if entries.len() != 1 || entries[0] != generation {
        bail!(
            "{} のframesに複数世代があります: {}",
            character_dir.display(),
            entries.join(", ")
        );
    }
    Ok(())
}

/// マニフェストが宣言する Live2D モデルのパス一覧（`live2d.model` と `live2d.models[].path`）。
///
/// 検証（実在確認）と同期（未取得サンプルの判定）が同じ定義を見るよう1か所にまとめる。
fn live2d_model_paths(manifest: &serde_json::Value) -> Result<Vec<String>> {
    let live2d = &manifest["live2d"];
    let mut models = Vec::new();
    if let Some(path) = live2d.get("model").and_then(serde_json::Value::as_str) {
        models.push(path.to_string());
    }
    if let Some(sets) = live2d.get("models").and_then(serde_json::Value::as_array) {
        for set in sets {
            let path = set
                .get("path")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            if path.is_empty() {
                bail!("Live2Dモデル世代にpathがありません");
            }
            models.push(path.to_string());
        }
    }
    if models.is_empty() {
        bail!("Live2Dモデル定義が空です");
    }
    Ok(models)
}

fn validate_live2d_model_sets(character_dir: &Path, manifest: &serde_json::Value) -> Result<()> {
    let models = live2d_model_paths(manifest)
        .with_context(|| format!("{} のLive2D定義", character_dir.display()))?;
    for model in models {
        if !character_dir.join(&model).is_file() {
            bail!(
                "{} のLive2Dモデル一式がありません: {model}",
                character_dir.display()
            );
        }
    }
    Ok(())
}

fn character_content_version(character_dir: &Path) -> Result<String> {
    let mut files = list_files(character_dir);
    files.sort();
    let mut hasher = Sha256::new();
    for file in files {
        let relative = file.strip_prefix(character_dir).unwrap_or(&file);
        hasher.update(relative.to_string_lossy().replace('\\', "/").as_bytes());
        hasher.update([0]);
        let mut bytes = fs::read(&file)?;
        if relative == Path::new("character.json") {
            let mut manifest: serde_json::Value = serde_json::from_slice(&bytes)?;
            if let Some(object) = manifest.as_object_mut() {
                object.remove("assetVersion");
            }
            bytes = serde_json::to_vec(&manifest)?;
        }
        hasher.update(bytes);
        hasher.update([0]);
    }
    Ok(hex::encode(hasher.finalize())[..16].to_string())
}

/// `ui/` を Tauri 配信用 `ui-dist/` へコピーする。
///
/// release では依存追加なしで HTML/CSS/JS の余分な空白とコメントを落とし、
/// 配布物から読みやすいソースを減らす（解析を不可能にするものではない）。
///
/// `beta` は開発中ベータ版のときだけ真にする。偽なら画面の BETA 表示を取り除く。
pub fn build_ui_dist(release: bool, beta: bool, force: bool) -> Result<()> {
    let root = repo_root();
    let src = root.join("ui");
    let dest = root.join("ui-dist");
    if !src.exists() {
        bail!("ui が見つかりません: {}", src.display());
    }
    let stats = sync_dir(&src, &dest, &|_| false, force)?;
    // READMEで実表示を検証した8形式の画像を、設定画面の選択カードにも使う。
    // 正本を二重管理せず、配信用生成物へだけまとめてコピーする。
    let preview_src = root.join("docs/images");
    let preview_dest = dest.join("assets/layout-previews");
    fs::create_dir_all(&preview_dest)?;
    for id in [
        "pet",
        "mascot",
        "chat",
        "bar",
        "workspace",
        "stream",
        "widget",
    ] {
        let source = preview_src.join(format!("layout-shot-{id}.webp"));
        if !source.exists() {
            bail!(
                "表示形式のプレビュー画像が見つかりません: {}",
                source.display()
            );
        }
        fs::copy(source, preview_dest.join(format!("{id}.webp")))?;
    }

    if !beta {
        strip_beta_badge(&dest)?;
    }

    if release {
        for file in list_files(&dest) {
            let ext = file
                .extension()
                .map(|e| e.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            let Ok(text) = fs::read_to_string(&file) else {
                continue;
            };
            let minified = match ext.as_str() {
                "html" => minify_html(&text),
                "css" => minify_css(&text),
                "js" => minify_js(&text),
                _ => continue,
            };
            fs::write(&file, minified)?;
        }
    }

    // 配信する中身が確定してから版を打つ（release の最小化後の内容でハッシュを取る）。
    stamp_asset_versions(&dest)?;

    log(
        "build-ui-dist",
        format!(
            "ui -> ui-dist ({}{}、コピー{}件、変更なし{}件、削除{}件)",
            if release { "release" } else { "dev" },
            if beta { " / beta" } else { "" },
            stats.copied_files,
            stats.unchanged_files,
            stats.removed_entries
        ),
    );
    Ok(())
}

/// 配信用 `index.html` から BETA 表示を取り除く。
///
/// BETA 表示は `ui/index.html` に直接書いてあり、`ui/` はどのビルドでもそのまま
/// `ui-dist/` へ写る。剥がさないと販売版にも「BETA」が出続けるため、ベータ以外の
/// ビルドはここで確実に落とす（表示の有無をビルド種別だけで決める）。
fn strip_beta_badge(dest: &std::path::Path) -> Result<()> {
    let index = dest.join("index.html");
    let text = fs::read_to_string(&index)
        .with_context(|| format!("index.html を読めません: {}", index.display()))?;
    let Some(stripped) = remove_beta_badge(&text) else {
        return Ok(()); // 目印が無いソース（将来 BETA 表示を廃止した場合）は何もしない
    };
    fs::write(&index, stripped)?;
    Ok(())
}

/// `<span class="beta-badge" …>BETA</span>` を1つ取り除く。見つからなければ `None`。
fn remove_beta_badge(text: &str) -> Option<String> {
    const OPEN: &str = "<span class=\"beta-badge\"";
    let start = text.find(OPEN)?;
    let end = text[start..].find("</span>")? + start + "</span>".len();
    let mut stripped = String::with_capacity(text.len());
    stripped.push_str(&text[..start]);
    stripped.push_str(text[end..].trim_start_matches(['\r', '\n', ' ', '\t']));
    Some(stripped)
}

/// HTML が読み込むローカルの CSS / JS へ、内容から決まる `?v=…` を付ける。
///
/// 手書きの版番号は付け忘れ・書き間違いが起きる（実際に `styles.css` と `renderer.js` で
/// 版が食い違っていた）。内容ハッシュにしておけば、変更したファイルだけが確実に読み直され、
/// webview のキャッシュに古い CSS が残らない。
fn stamp_asset_versions(dest: &std::path::Path) -> Result<()> {
    for file in list_files(dest) {
        if file.extension().map(|e| e != "html").unwrap_or(true) {
            continue;
        }
        let Some(dir) = file.parent().map(|dir| dir.to_path_buf()) else {
            continue;
        };
        let text = fs::read_to_string(&file)?;
        let stamped = rewrite_asset_refs(&text, &|relative| {
            let target = dir.join(relative);
            fs::read(&target).ok().map(|bytes| content_version(&bytes))
        });
        if stamped != text {
            fs::write(&file, stamped)?;
        }
    }
    Ok(())
}

/// 内容から決まる短い版文字列（FNV-1a。外部クレートを増やさない）。
fn content_version(bytes: &[u8]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{:08x}", (hash >> 32) as u32)
}

/// HTML 内の `href="…"` / `src="…"` のうち、同梱の CSS / JS を指すものへ版を付け直す。
fn rewrite_asset_refs(html: &str, version_of: &dyn Fn(&str) -> Option<String>) -> String {
    const ATTRIBUTES: [&str; 2] = ["href=\"", "src=\""];
    let mut out = String::with_capacity(html.len() + 64);
    let mut rest = html;
    loop {
        let Some((index, attribute)) = ATTRIBUTES
            .iter()
            .filter_map(|attribute| rest.find(attribute).map(|index| (index, *attribute)))
            .min_by_key(|(index, _)| *index)
        else {
            out.push_str(rest);
            return out;
        };
        let value_start = index + attribute.len();
        out.push_str(&rest[..value_start]);
        let after = &rest[value_start..];
        let Some(end) = after.find('"') else {
            out.push_str(after);
            return out;
        };
        out.push_str(&stamped_reference(&after[..end], version_of));
        out.push('"');
        rest = &after[end + 1..];
    }
}

/// 参照1つ分の書き換え。外部 URL・絶対パス・CSS/JS 以外はそのまま返す。
fn stamped_reference(value: &str, version_of: &dyn Fn(&str) -> Option<String>) -> String {
    let path = value.split(['?', '#']).next().unwrap_or(value);
    let lower = path.to_lowercase();
    if !(lower.ends_with(".css") || lower.ends_with(".js")) {
        return value.to_string();
    }
    if path.starts_with('/') || path.contains("://") || lower.starts_with("data:") {
        return value.to_string();
    }
    match version_of(path) {
        Some(version) => format!("{path}?v={version}"),
        None => value.to_string(),
    }
}

fn minify_html(text: &str) -> String {
    let without_comments = strip_between(text, "<!--", "-->");
    let mut out = String::with_capacity(without_comments.len());
    let chars = without_comments.chars();
    let mut pending_space = false;
    for ch in chars {
        if ch.is_whitespace() {
            pending_space = true;
            continue;
        }
        if pending_space {
            // タグ間の空白は落とし、それ以外は1つの空白へまとめる。
            if !(out.is_empty() || ch == '<' && out.ends_with('>')) {
                out.push(' ');
            }
            pending_space = false;
        }
        out.push(ch);
    }
    out.trim().to_string()
}

fn minify_css(text: &str) -> String {
    let without_comments = strip_between(text, "/*", "*/");
    let mut collapsed = String::with_capacity(without_comments.len());
    let mut in_space = false;
    for ch in without_comments.chars() {
        if ch.is_whitespace() {
            in_space = true;
            continue;
        }
        if in_space && !collapsed.is_empty() {
            collapsed.push(' ');
        }
        in_space = false;
        collapsed.push(ch);
    }
    // 記号の前後の空白を落とす。
    let mut out = String::with_capacity(collapsed.len());
    // `calc(100% + 16px)` の + / - は前後の空白が必須。ここで削ると宣言全体が
    // 無効になり、release 配布版だけキャラ幅が0になるため演算子は対象に含めない。
    let symbols = ['{', '}', ':', ';', ',', '>', '~'];
    let chars: Vec<char> = collapsed.chars().collect();
    let mut index = 0;
    while index < chars.len() {
        let ch = chars[index];
        if ch == ' ' {
            let prev = out.chars().last();
            let next = chars.get(index + 1).copied();
            let touching_symbol = prev.is_some_and(|c| symbols.contains(&c))
                || next.is_some_and(|c| symbols.contains(&c));
            if touching_symbol {
                index += 1;
                continue;
            }
        }
        out.push(ch);
        index += 1;
    }
    out.replace(";}", "}").trim().to_string()
}

fn minify_js(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// `open` と `close` に囲まれた範囲を落とす（コメント除去用の素朴な実装）。
fn strip_between(text: &str, open: &str, close: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find(open) {
        out.push_str(&rest[..start]);
        let after = &rest[start + open.len()..];
        match after.find(close) {
            Some(end) => rest = &after[end + close.len()..],
            // 閉じが無い場合は残り全部がコメント扱い。
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

/// 前回起動の残存プロセスを停止する。
///
/// 残っていると同梱 DLL を掴んだままになり、ビルドが `os error 32` で失敗したり、
/// 新しいインスタンスの WebView 生成と競合して起動直後に落ちたりする。
/// 名前一致だけで止めてよい対象。PicoAgent 管理下の実行ファイル名に限る。
/// `node` のように他アプリと共用される名前をここへ足さない（別の経路で判別して止める）。
const NAME_MATCH_STOP_TARGETS: [&str; 3] = ["picoagent", "llama-server", "whisper-cli"];

pub fn stop_stale() -> Result<()> {
    // PicoAgent 管理下のものだけを対象にする（同名の無関係プロセスを巻き込まない）。
    let targets = NAME_MATCH_STOP_TARGETS;
    let mut stopped = Vec::new();

    for name in targets {
        let killed = if cfg!(windows) {
            Command::new("taskkill")
                .args(["/IM", &format!("{name}.exe"), "/T", "/F"])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .map(|status| status.success())
                .unwrap_or(false)
        } else {
            // 完全一致だけを対象にする（-x）。
            Command::new("pkill")
                .args(["-x", name])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .map(|status| status.success())
                .unwrap_or(false)
        };
        if killed {
            stopped.push(name);
        }
    }

    // Aivis は PicoAgent 管理下のパスから起動したものだけを止める。
    stop_managed_aivis(&mut stopped);
    // 撮影・検査ツールの node も、このリポジトリの経路から起動したものだけを止める。
    stop_managed_screenshot_node(&mut stopped);

    if stopped.is_empty() {
        log("stop-stale", "停止対象の残存プロセスはありません");
    } else {
        log(
            "stop-stale",
            format!("停止しました: {}", stopped.join(", ")),
        );
    }
    Ok(())
}

#[cfg(unix)]
fn stop_managed_aivis(stopped: &mut Vec<&'static str>) {
    // 実行パスに PicoAgent 管理下の bundled/aivis を含むものだけを止める。
    let ok = Command::new("pkill")
        .args(["-f", "PicoAgent.*bundled.*aivis.*run"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false);
    if ok {
        stopped.push("aivis");
    }
}

#[cfg(not(unix))]
fn stop_managed_aivis(stopped: &mut Vec<&'static str>) {
    let ok = Command::new("taskkill")
        .args(["/IM", "run.exe", "/T", "/F"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false);
    if ok {
        stopped.push("aivis");
    }
}

/// 撮影・検査ツールが残した `node` の目印。名前だけで止めると Codex の MCP サーバや
/// Adobe Creative Cloud の `node.exe` まで巻き込むため、必ずこの経路と突き合わせる。
///
/// コマンドラインは `node tools/screenshots/shoot.mjs` のような相対・スラッシュ表記にも
/// 絶対・円記号表記にもなる。区切りをスラッシュへ揃えてから含有判定するため、
/// ここは絶対パスではなくフォルダ構成そのものを目印にする。
const SCREENSHOT_TOOL_NEEDLE: &str = "tools/screenshots/";

#[cfg(unix)]
fn stop_managed_screenshot_node(stopped: &mut Vec<&'static str>) {
    let ok = Command::new("pkill")
        .args(["-f", SCREENSHOT_TOOL_NEEDLE])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false);
    if ok {
        stopped.push("screenshot-node");
    }
}

#[cfg(not(unix))]
fn stop_managed_screenshot_node(stopped: &mut Vec<&'static str>) {
    // コマンドラインへ撮影ツールの経路を含む node だけを、プロセスツリーごと止める。
    // 円記号の区切りをスラッシュへ揃えてから判定し、相対・絶対どちらの起動でも拾う。
    let script = format!(
        "$needle = '{SCREENSHOT_TOOL_NEEDLE}';          Get-CimInstance Win32_Process -Filter \"Name='node.exe'\" |          Where-Object {{ $_.CommandLine -and $_.CommandLine.Replace('\\', '/').Contains($needle) }} |          ForEach-Object {{ taskkill /PID $_.ProcessId /T /F > $null 2>&1; 'stopped' }}",
    );
    let stopped_any = Command::new("powershell")
        .args(["-NoProfile", "-Command", &script])
        .stderr(std::process::Stdio::null())
        .output()
        .map(|output| String::from_utf8_lossy(&output.stdout).contains("stopped"))
        .unwrap_or(false);
    if stopped_any {
        stopped.push("screenshot-node");
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{
        apply_delivery_plan, build_delivery_plan, character_content_version,
        character_manifests_equivalent, content_version, delivery_image_matches,
        drop_characters_from_index, file_sha256, image_dimensions, is_character_source_only,
        remove_beta_badge, remove_source_layers_from_runtime_copy, rewrite_asset_refs,
        stamp_asset_versions, stamp_character_asset_versions, unfetched_live2d_characters,
        validate_character_asset_generation,
    };

    fn write_test_webp(path: &std::path::Path, width: u32, height: u32, seed: u8) {
        let mut pixels = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            for x in 0..width {
                pixels.extend_from_slice(&[
                    (x * 40 + 20) as u8,
                    (y % 251) as u8,
                    180_u8.wrapping_add(seed),
                    if y % 3 == 0 { 64 } else { 255 },
                ]);
            }
        }
        let mut encoded = Vec::new();
        image::codecs::webp::WebPEncoder::new_lossless(&mut encoded)
            .encode(&pixels, width, height, image::ExtendedColorType::Rgba8)
            .unwrap();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, encoded).unwrap();
    }

    #[test]
    fn 制作用素材は配信コピーから外す() {
        assert!(is_character_source_only(std::path::Path::new(
            "characters/mugi/expression-patches/blink.png"
        )));
        assert!(is_character_source_only(std::path::Path::new(
            "characters/hiyori/live2d-src/source.psd"
        )));
        assert!(is_character_source_only(std::path::Path::new(
            "characters/mugi/runtime-layers.part/base.webp"
        )));
        assert!(is_character_source_only(std::path::Path::new(
            "characters/mugi/frames-deadbeef.part/base.webp"
        )));
        assert!(!is_character_source_only(std::path::Path::new(
            "characters/mugi/frames/idle.png"
        )));

        let ui = crate::util::repo_root().join("ui");
        let mut stack = vec![ui];
        let mut references = Vec::new();
        while let Some(dir) = stack.pop() {
            for entry in fs::read_dir(dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    if path.file_name().and_then(|name| name.to_str()) != Some("assets") {
                        stack.push(path);
                    }
                } else if matches!(
                    path.extension().and_then(|ext| ext.to_str()),
                    Some("js" | "html" | "css")
                ) && fs::read_to_string(&path)
                    .is_ok_and(|text| text.contains("expression-patches"))
                {
                    references.push(path);
                }
            }
        }
        let mut character_stack = vec![crate::util::repo_root().join("characters")];
        while let Some(dir) = character_stack.pop() {
            for entry in fs::read_dir(dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    character_stack.push(path);
                } else if path.extension().and_then(|ext| ext.to_str()) == Some("json")
                    && fs::read_to_string(&path)
                        .is_ok_and(|text| text.contains("expression-patches"))
                {
                    references.push(path);
                }
            }
        }
        assert!(
            references.is_empty(),
            "実行UIから制作用素材を参照しています: {references:?}"
        );
    }

    /// `characters/<id>/` を1体ぶん作る。`models` は `character.json` へ書く宣言、
    /// `present` は実際に置くファイル（宣言だけあってファイルが無い状態を作れるようにする）。
    fn live2d_character(src: &std::path::Path, id: &str, models: &[&str], present: &[&str]) {
        let dir = src.join(id);
        fs::create_dir_all(&dir).unwrap();
        let declared = models
            .iter()
            .map(|model| format!(r#"{{"path":"{model}"}}"#))
            .collect::<Vec<_>>()
            .join(",");
        fs::write(
            dir.join("character.json"),
            format!(r#"{{"id":"{id}","live2d":{{"models":[{declared}]}}}}"#),
        )
        .unwrap();
        for model in present {
            let path = dir.join(model);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, b"model").unwrap();
        }
    }

    /// `characters/*/live2d/` は再配布条件があるため Git 管理外で、clone 直後には無い。
    /// ここで外し損ねると同期が中断し、`ui-dist/` が作られずアプリが起動しなくなる。
    #[test]
    fn モデル未取得のlive2dサンプルは同期から外す() {
        let root =
            std::env::temp_dir().join(format!("picoagent-unfetched-live2d-{}", std::process::id()));
        fs::remove_dir_all(&root).ok();
        let src = root.join("characters");
        fs::create_dir_all(&src).unwrap();

        // 宣言だけあってモデルが1つも無い＝未取得。
        live2d_character(&src, "hiyori", &["live2d/hiyori.model3.json"], &[]);
        // 取得済み＝外さない。
        live2d_character(
            &src,
            "mugi-live2d",
            &["live2d/mugi.model3.json"],
            &["live2d/mugi.model3.json"],
        );
        // 一部だけ欠けているのは壊れた状態なので、外さず検証側のエラーに任せる。
        live2d_character(
            &src,
            "half",
            &["live2d/sdk4/a.model3.json", "live2d/sdk5/b.model3.json"],
            &["live2d/sdk4/a.model3.json"],
        );
        // Live2D を宣言しない PNG キャラは対象外。
        let png = src.join("mugi");
        fs::create_dir_all(&png).unwrap();
        fs::write(png.join("character.json"), r#"{"id":"mugi"}"#).unwrap();

        assert_eq!(unfetched_live2d_characters(&src).unwrap(), vec!["hiyori"]);
        fs::remove_dir_all(&root).unwrap();
    }

    /// 外したキャラを一覧に残すと、設定画面が「選べるのに読み込めないキャラ」を見せてしまう。
    #[test]
    fn 外したキャラは一覧と既定からも消す() {
        let dest =
            std::env::temp_dir().join(format!("picoagent-drop-index-{}", std::process::id()));
        fs::remove_dir_all(&dest).ok();
        fs::create_dir_all(&dest).unwrap();
        fs::write(
            dest.join("index.json"),
            r#"{"default":"hiyori","characters":[{"id":"hiyori"},{"id":"mugi"},{"id":"rin"}]}"#,
        )
        .unwrap();

        drop_characters_from_index(&dest, &["hiyori".to_string()]).unwrap();
        let index: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(dest.join("index.json")).unwrap()).unwrap();
        let ids = index["characters"]
            .as_array()
            .unwrap()
            .iter()
            .map(|character| character["id"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>();
        assert_eq!(ids, ["mugi", "rin"]);
        // 既定キャラを外したときだけ、残っている先頭へ寄せる。
        assert_eq!(index["default"].as_str(), Some("mugi"));

        // 全部外すと選べるキャラが居なくなるため、黙って空の一覧を配らない。
        assert!(drop_characters_from_index(&dest, &["mugi".into(), "rin".into()]).is_err());
        fs::remove_dir_all(&dest).unwrap();
    }

    #[test]
    fn 同梱のcssとjsへ内容版を付ける() {
        let html =
            r#"<link rel="stylesheet" href="styles.css" /><script src="renderer.js"></script>"#;
        let stamped = rewrite_asset_refs(html, &|relative| match relative {
            "styles.css" => Some("aaaaaaaa".to_string()),
            "renderer.js" => Some("bbbbbbbb".to_string()),
            _ => None,
        });
        assert_eq!(
            stamped,
            r#"<link rel="stylesheet" href="styles.css?v=aaaaaaaa" /><script src="renderer.js?v=bbbbbbbb"></script>"#
        );
    }

    #[test]
    fn 配信htmlへ実ファイルの内容版を打てる() {
        let root =
            std::env::temp_dir().join(format!("picoagent-ui-version-{}", std::process::id()));
        fs::remove_dir_all(&root).ok();
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("styles.css"), "body{}").unwrap();
        fs::write(
            root.join("index.html"),
            r#"<link rel="stylesheet" href="styles.css">"#,
        )
        .unwrap();
        stamp_asset_versions(&root).unwrap();
        let html = fs::read_to_string(root.join("index.html")).unwrap();
        assert!(html.contains("styles.css?v="));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn キャラmanifestは自動版だけの差なら再コピーしない() {
        let root = std::env::temp_dir().join(format!(
            "picoagent-manifest-equivalence-{}",
            std::process::id()
        ));
        fs::remove_dir_all(&root).ok();
        fs::create_dir_all(&root).unwrap();
        let source = root.join("character.json");
        let target = root.join("target/character.json");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(&source, r#"{"id":"sample","assetVersion":"source"}"#).unwrap();
        fs::write(&target, r#"{"id":"sample","assetVersion":"manual"}"#).unwrap();
        assert!(!character_manifests_equivalent(&source, &target).unwrap());
        fs::write(&target, r#"{"id":"sample","assetVersion":"auto-1234"}"#).unwrap();
        assert!(character_manifests_equivalent(&source, &target).unwrap());
        fs::write(
            &target,
            r#"{"id":"sample","assetVersion":"auto-1234","deliveryImages":{"policy":"test"}}"#,
        )
        .unwrap();
        assert!(character_manifests_equivalent(&source, &target).unwrap());
        fs::write(&target, r#"{"id":"changed","assetVersion":"auto-1234"}"#).unwrap();
        assert!(!character_manifests_equivalent(&source, &target).unwrap());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn 高精細な正本を保ったまま配信用webpだけを縮小する() {
        let root = std::env::temp_dir().join(format!(
            "picoagent-delivery-image-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let src = root.join("characters");
        let dest = root.join("ui/assets/characters");
        let source_character = src.join("sample");
        let target_character = dest.join("sample");
        fs::create_dir_all(&source_character).unwrap();
        fs::create_dir_all(&target_character).unwrap();
        let manifest = r#"{
          "id":"sample",
          "animationQualityMode":"sharp-expression",
          "runtimeLayers":{"base":"runtime/base.webp"},
          "animations":{"idle":{"framePaths":["runtime/base.webp"]}}
        }"#;
        fs::write(source_character.join("character.json"), manifest).unwrap();
        fs::write(target_character.join("character.json"), manifest).unwrap();
        let source_image = source_character.join("runtime/base.webp");
        let target_image = target_character.join("runtime/base.webp");
        write_test_webp(&source_image, 2, 1801, 0);
        fs::create_dir_all(target_image.parent().unwrap()).unwrap();
        fs::copy(&source_image, &target_image).unwrap();
        let source_hash = file_sha256(&source_image).unwrap();

        let plan = build_delivery_plan(&src, &dest, &|_| true).unwrap();
        assert_eq!(plan.by_source.len(), 1);
        assert!(!plan.by_source.values().next().unwrap().reusable);
        let changed = apply_delivery_plan(&dest, &plan).unwrap();
        assert_eq!(changed, ["sample".to_string()].into_iter().collect());
        assert_eq!(image_dimensions(&source_image).unwrap(), (2, 1801));
        assert_eq!(file_sha256(&source_image).unwrap(), source_hash);
        assert!(delivery_image_matches(&target_image, 2, 1800).unwrap());
        let alpha = image::ImageReader::open(&target_image)
            .unwrap()
            .with_guessed_format()
            .unwrap()
            .decode()
            .unwrap()
            .to_rgba8()
            .pixels()
            .map(|pixel| pixel.0[3])
            .collect::<Vec<_>>();
        assert!(alpha.iter().any(|value| *value < 255));
        assert!(alpha.contains(&255));
        let target_manifest: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(target_character.join("character.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            target_manifest["deliveryImages"]["policy"].as_str(),
            Some(super::DELIVERY_IMAGE_POLICY)
        );
        assert!(serde_json::from_str::<serde_json::Value>(
            &fs::read_to_string(source_character.join("character.json")).unwrap()
        )
        .unwrap()
        .get("deliveryImages")
        .is_none());

        let reusable = build_delivery_plan(&src, &dest, &|_| true).unwrap();
        assert!(reusable.by_source.values().next().unwrap().reusable);
        assert!(apply_delivery_plan(&dest, &reusable).unwrap().is_empty());
        write_test_webp(&source_image, 2, 1801, 1);
        let changed_source = build_delivery_plan(&src, &dest, &|_| true).unwrap();
        assert!(!changed_source.by_source.values().next().unwrap().reusable);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn 通常品質と配信上限内の画像は縮小しない() {
        let root = std::env::temp_dir().join(format!(
            "picoagent-delivery-image-skip-{}",
            std::process::id()
        ));
        fs::remove_dir_all(&root).ok();
        let src = root.join("characters");
        let dest = root.join("ui/assets/characters");
        for (id, quality, height) in [
            ("normal", "standard", 1801),
            ("within-limit", "sharp-expression", 1800),
        ] {
            let character = src.join(id);
            fs::create_dir_all(&character).unwrap();
            fs::write(
                character.join("character.json"),
                format!(
                    r#"{{"id":"{id}","animationQualityMode":"{quality}","animations":{{"idle":{{"framePaths":["frame.webp"]}}}}}}"#
                ),
            )
            .unwrap();
            write_test_webp(&character.join("frame.webp"), 2, height, 0);
        }
        let plan = build_delivery_plan(&src, &dest, &|_| true).unwrap();
        assert!(plan.by_source.is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn 手書きの版と外部参照は壊さない() {
        // 手書きの `?v=…` は内容版へ置き換える（付け忘れ・食い違いを残さない）。
        let stamped =
            rewrite_asset_refs(r#"<script src="renderer.js?v=20260807"></script>"#, &|_| {
                Some("cccccccc".to_string())
            });
        assert_eq!(stamped, r#"<script src="renderer.js?v=cccccccc"></script>"#);

        // 外部 URL・存在しないファイル・CSS/JS 以外は素通し。
        let untouched =
            r#"<a href="https://example.com/app.js"><img src="icon.png"><link href="missing.css">"#;
        assert_eq!(
            rewrite_asset_refs(untouched, &|relative| (relative == "styles.css")
                .then(|| "dddddddd".to_string())),
            untouched
        );
    }

    /// BETA 表示は開発中ベータ版だけの目印。`ui/index.html` に直接書いてあるため、
    /// 剥がす処理が実際の記述と噛み合っていないと販売版へそのまま出てしまう。
    #[test]
    fn ベータ表示は販売ビルドから取り除ける() {
        let index = fs::read_to_string(crate::util::repo_root().join("ui/index.html")).unwrap();
        assert!(
            index.contains("beta-badge"),
            "ui/index.html に BETA 表示がありません。廃止したなら strip_beta_badge も消すこと"
        );
        let stripped = remove_beta_badge(&index).expect("BETA 表示を取り除けません");
        assert!(!stripped.contains("beta-badge"));
        assert!(remove_beta_badge(&stripped).is_none());
        // 目印が無い HTML は書き換え対象にしない。
        assert!(remove_beta_badge("<main></main>").is_none());
    }

    #[test]
    fn 内容が変われば版も変わる() {
        assert_eq!(content_version(b"body{}"), content_version(b"body{}"));
        assert_ne!(content_version(b"body{}"), content_version(b"body{ }"));
        assert_eq!(content_version(b"body{}").len(), 8);
    }

    #[test]
    fn css圧縮でcalc演算子の空白を保つ() {
        assert_eq!(
            super::minify_css(".a { right: calc(var(--width) + 16px); }").as_str(),
            ".a{right:calc(var(--width) + 16px)}"
        );
    }

    #[test]
    fn キャラ素材更新でmanifestの自動版が変わる() {
        let root = std::env::temp_dir().join(format!(
            "picoagent-character-version-{}",
            std::process::id()
        ));
        let character = root.join("sample");
        fs::create_dir_all(&character).unwrap();
        fs::write(
            character.join("character.json"),
            r#"{"id":"sample","assetVersion":"manual","frameGeneration":"current","animations":{"idle":{"framePaths":["frames/current/a.png"]}}}"#,
        )
        .unwrap();
        fs::create_dir_all(character.join("frames/current")).unwrap();
        fs::write(character.join("frames/current/a.png"), b"frame").unwrap();
        fs::write(character.join("preview.png"), b"old image").unwrap();
        let old = character_content_version(&character).unwrap();
        fs::write(character.join("preview.png"), b"new image").unwrap();
        let new = character_content_version(&character).unwrap();
        assert_ne!(old, new);

        stamp_character_asset_versions(&root, None).unwrap();
        let manifest: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(character.join("character.json")).unwrap())
                .unwrap();
        assert!(manifest["assetVersion"]
            .as_str()
            .unwrap()
            .starts_with("auto-"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn pngキャラは単一フレーム世代だけを許可する() {
        let root =
            std::env::temp_dir().join(format!("picoagent-frame-generation-{}", std::process::id()));
        let character = root.join("sample");
        fs::create_dir_all(character.join("frames/current")).unwrap();
        fs::write(character.join("frames/current/a.png"), b"image").unwrap();
        let manifest: serde_json::Value = serde_json::from_str(
            r#"{"frameGeneration":"current","animations":{"idle":{"framePaths":["frames/current/a.png"]}}}"#,
        )
        .unwrap();
        validate_character_asset_generation(&character, &manifest).unwrap();

        fs::create_dir_all(character.join("frames/old")).unwrap();
        assert!(validate_character_asset_generation(&character, &manifest).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn 同梱2dの実行画像はlossless_webp単一世代にする() {
        let characters = crate::util::repo_root().join("characters");
        for entry in fs::read_dir(characters).unwrap().flatten() {
            let character = entry.path();
            let manifest_path = character.join("character.json");
            if !manifest_path.is_file() {
                continue;
            }
            let manifest: serde_json::Value =
                serde_json::from_str(&fs::read_to_string(manifest_path).unwrap()).unwrap();
            if manifest.get("live2d").is_some() {
                continue;
            }
            assert_eq!(
                manifest
                    .get("frameEncoding")
                    .and_then(serde_json::Value::as_str),
                Some("webp-lossless-v1"),
                "{} のフレームがlossless WebPではありません",
                character.display()
            );
            assert_eq!(
                manifest
                    .get("runtimeLayerEncoding")
                    .and_then(serde_json::Value::as_str),
                Some("webp-lossless-v1"),
                "{} の実行レイヤーがlossless WebPではありません",
                character.display()
            );
            validate_character_asset_generation(&character, &manifest).unwrap();
            for name in super::FULL_MOTION_LAYERS {
                assert!(
                    character
                        .join("layers")
                        .join(format!("{name}.png"))
                        .is_file(),
                    "制作正本PNGを消しています: {}/layers/{name}.png",
                    character.display()
                );
                let runtime = manifest["runtimeLayers"][*name].as_str().unwrap();
                assert!(super::is_lossless_webp(&character.join(runtime)).unwrap());
            }
            assert_eq!(
                fs::read_dir(character.join("frames"))
                    .unwrap()
                    .flatten()
                    .flat_map(|generation| fs::read_dir(generation.path()).unwrap().flatten())
                    .filter(
                        |frame| frame.path().extension().and_then(|ext| ext.to_str())
                            == Some("png")
                    )
                    .count(),
                0,
                "{} の実行フレームにPNGが残っています",
                character.display()
            );
        }
    }

    #[test]
    fn フルモーション宣言は全共通レイヤーを必須にする() {
        let root =
            std::env::temp_dir().join(format!("picoagent-full-motion-{}", std::process::id()));
        let character = root.join("sample");
        fs::create_dir_all(character.join("layers")).unwrap();
        let manifest: serde_json::Value =
            serde_json::from_str(r#"{"motionProfile":"full"}"#).unwrap();
        assert!(super::validate_full_motion_pack(&character, &manifest).is_err());
        for name in super::FULL_MOTION_LAYERS {
            fs::write(
                character.join("layers").join(format!("{name}.png")),
                b"image",
            )
            .unwrap();
        }
        super::validate_full_motion_pack(&character, &manifest).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn 配信コピーはlossless_webpレイヤーだけを残す() {
        let root = std::env::temp_dir().join(format!(
            "picoagent-runtime-webp-layers-{}",
            std::process::id()
        ));
        fs::remove_dir_all(&root).ok();
        fs::create_dir_all(root.join("layers")).unwrap();
        fs::create_dir_all(root.join("runtime-layers")).unwrap();
        let mut runtime_layers = serde_json::Map::new();
        for name in super::FULL_MOTION_LAYERS {
            fs::write(root.join("layers").join(format!("{name}.png")), b"source").unwrap();
            fs::write(
                root.join("runtime-layers").join(format!("{name}.webp")),
                b"RIFF\0\0\0\0WEBPVP8L",
            )
            .unwrap();
            runtime_layers.insert(
                (*name).to_string(),
                serde_json::Value::String(format!("runtime-layers/{name}.webp")),
            );
        }
        let manifest = serde_json::json!({
            "runtimeLayers": runtime_layers,
            "runtimeLayerEncoding": "webp-lossless-v1"
        });
        remove_source_layers_from_runtime_copy(&root, &manifest).unwrap();
        assert!(!root.join("layers").exists());
        assert!(root.join("runtime-layers/base.webp").is_file());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn lossy_webpをlossless宣言では通さない() {
        let root = std::env::temp_dir().join(format!(
            "picoagent-runtime-lossy-webp-{}",
            std::process::id()
        ));
        fs::remove_dir_all(&root).ok();
        fs::create_dir_all(root.join("layers")).unwrap();
        fs::create_dir_all(root.join("runtime-layers")).unwrap();
        let mut runtime_layers = serde_json::Map::new();
        for name in super::FULL_MOTION_LAYERS {
            fs::write(
                root.join("runtime-layers").join(format!("{name}.webp")),
                b"RIFF\0\0\0\0WEBPVP8 ",
            )
            .unwrap();
            runtime_layers.insert(
                (*name).to_string(),
                serde_json::Value::String(format!("runtime-layers/{name}.webp")),
            );
        }
        let manifest = serde_json::json!({
            "runtimeLayers": runtime_layers,
            "runtimeLayerEncoding": "webp-lossless-v1"
        });
        assert!(remove_source_layers_from_runtime_copy(&root, &manifest).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn 高精細表情キャラは全レイヤーの最低寸法を満たす() {
        let root =
            std::env::temp_dir().join(format!("picoagent-sharp-size-{}", std::process::id()));
        let character = root.join("sample");
        fs::create_dir_all(character.join("layers")).unwrap();
        let mut header = Vec::from(b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".as_slice());
        header.extend_from_slice(&890_u32.to_be_bytes());
        header.extend_from_slice(&1800_u32.to_be_bytes());
        for name in super::FULL_MOTION_LAYERS {
            fs::write(
                character.join("layers").join(format!("{name}.png")),
                &header,
            )
            .unwrap();
        }
        let manifest: serde_json::Value =
            serde_json::from_str(r#"{"animationQualityMode":"sharp-expression"}"#).unwrap();
        assert!(super::validate_sharp_expression_pack(&character, &manifest).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn 名前一致で止める対象へ他アプリと共用する名前を入れない() {
        // `node` は Codex の MCP サーバや Adobe Creative Cloud も使う。名前だけで
        // 止めると稼働中アプリを巻き込むため、経路一致の停止経路だけを使う。
        for name in super::NAME_MATCH_STOP_TARGETS {
            assert_ne!(name, "node");
            assert_ne!(name, "powershell");
        }
    }

    #[test]
    fn 撮影ツールのnode判定は区切りを揃えた経路で行う() {
        // 実際の起動は相対・スラッシュ、絶対・円記号のどちらもある。
        let needle = super::SCREENSHOT_TOOL_NEEDLE;
        assert_eq!(needle, "tools/screenshots/");
        for command in [
            "node tools/screenshots/shoot.mjs",
            "C:\\repo\\tools\\screenshots\\shoot.mjs",
        ] {
            assert!(
                command.replace('\\', "/").contains(needle),
                "拾えません: {command}"
            );
        }
        // 無関係な node は拾わない。
        for command in ["node ./mcp/server.mjs", "C:\\Adobe\\js\\main.js"] {
            assert!(
                !command.replace('\\', "/").contains(needle),
                "誤検出: {command}"
            );
        }
    }
}
