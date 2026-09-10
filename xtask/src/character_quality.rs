// xtask/src/character_quality.rs - 通常2Dキャラクターの制作正本を画素単位で検査する品質ゲート。
//
// `assets.rs` の同期時検証が見るのは「必要なファイルが揃っているか」「寸法と形式が
// 条件を満たすか」までで、画素の中身は見ていない。そのため次の事故は素通りしていた。
//
//   - 画像生成サービスが返した全身画像をそのまま差分レイヤーへ採用し、
//     状態切替で服・髪・体まで変わる
//   - 閉じ目の不透明度を下げただけの半目
//   - 差分がまったく無い（＝宣言されているのに動かない）レイヤー
//   - レイヤーごとに独立して再サンプリングした結果、顔以外の輪郭が状態切替でずれる
//   - クロマ背景の抜き残しでできた矩形、キャンバス端で切れた被写体
//   - 既定構図の表示倍率で原寸を割っていて、どう調整してもぼける原画
//
// AIによる自動生成を増やすほど、これらは人の目視より先に機械で止める必要がある。
//
// ## 判定の考え方
//
// 差分レイヤーは「顔の局所差分」であるべきなので、**顔より下で、輪郭でもない画素**が
// どれだけ書き換わったかを主指標にする。単純な変更面積比では、正しく作られた素材でも
// 輪郭のサブピクセル差で数%動いてしまい、全身の描き直しと区別できない。
//
// ## ベースライン
//
// 既存キャラクターには、この検査を導入する前に作られた素材が含まれる。それらを
// いきなり不合格にすると誰もビルドできないため、導入時点の指摘を
// `characters/quality-baseline.json` へ記録し、**新しい種類またはレイヤーの指摘だけ**を
// 失敗として扱う。既知の負債は毎回一覧に出して、解消したらベースラインから外す。

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use image::RgbaImage;
use serde_json::{json, Value};

use crate::util::{log, repo_root};

const BASE_LAYER: &str = "base";
const DIFFERENTIAL_LAYERS: [&str; 8] = [
    "blink_half",
    "blink_closed",
    "mouth_closed",
    "mouth_open",
    "gaze_left",
    "gaze_right",
    "expression_happy",
    "expression_sad",
];

/// 実行画像の縦上限。`assets.rs` の `DELIVERY_IMAGE_MAX_HEIGHT` と同じ値で、
/// 配信時にここまで縮むため、表示倍率の余裕はこの高さで判定する。
const DELIVERY_IMAGE_MAX_HEIGHT: u32 = 1800;

/// キャラクター枠の CSS 高さ（`ui/styles.css` の `--char-frame-height`）。
/// 立ち絵は縦長で `object-fit: contain` するため、実表示画素はこの高さ×倍率で決まる。
const CHARACTER_FRAME_HEIGHT: f64 = 315.0;

/// 原寸割れの判定に使う想定デバイス比。
const ASSUMED_DEVICE_PIXEL_RATIO: f64 = 2.0;

/// 画素が「変わった」とみなすチャンネル差の下限。この値以下は変化なしとして数えない。
/// lossless変換とPNG往復では差が0になるため、ここに残るのは実際の描き変えか
/// 再サンプリングのにじみだけになる。
const CHANGE_THRESHOLD: u8 = 8;

/// 被写体上端からこの比率までを顔とみなす。胸上・上半身のどちらの構図でも、
/// 目・口・眉はこの帯に収まる。
const FACE_BAND_RATIO: f64 = 0.35;

/// 輪郭とみなす輝度勾配。アンチエイリアス縁と、髪・服の内部線を拾う。
const CONTOUR_GRADIENT: f64 = 16.0;

/// 顔より下かつ輪郭でもない画素の書き換え上限（被写体画素に対する比）。
/// 同梱キャラクターの実測では、局所パッチで作った素材が最大1.05%（rin-tha3の視線）、
/// 全身を作り直した素材が最小3.00%（rinの視線）で、しきい値はその間に置いてある。
const OUTSIDE_FACE_LIMIT: f64 = 0.02;

/// 顔より下の輪郭が動いた量の上限（被写体画素に対する比）。
/// レイヤーを個別に再サンプリングすると必ずここを超える。
const CONTOUR_DRIFT_LIMIT: f64 = 0.001;

/// 直線的なアルファ境界を矩形とみなす長さ（被写体高さに対する比）。
///
/// 実測値: 同梱キャラクターの立ち絵は0.010〜0.026、数学的に完全な楕円でも0.139、
/// 背景板や貼り付けた矩形は1.000になる。滑らかな輪郭を巻き込まない位置に置く。
const RECTANGLE_EDGE_RATIO: f64 = 0.35;

/// 被写体が小さいときに、わずかな直線で誤検出しないための下限行数。
const RECTANGLE_EDGE_MINIMUM_ROWS: u32 = 16;

/// 生成元より大きい被写体を許す倍率。丸め誤差だけを吸収する。
const GENERATOR_UPSCALE_TOLERANCE: f64 = 1.02;

const BASELINE_FILE: &str = "characters/quality-baseline.json";

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Severity {
    Fail,
    Warn,
}

impl Severity {
    fn label(self) -> &'static str {
        match self {
            Severity::Fail => "不合格",
            Severity::Warn => "注意",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Finding {
    pub severity: Severity,
    pub character: String,
    pub check: &'static str,
    /// 対象レイヤー名。キャラクター全体の指摘では空。
    pub layer: String,
    pub message: String,
}

impl Finding {
    /// ベースライン照合用の安定した鍵。文言を変えても一致し続けるよう、
    /// 数値を含むメッセージは鍵に入れない。
    fn key(&self) -> String {
        if self.layer.is_empty() {
            format!("{}/{}", self.character, self.check)
        } else {
            format!("{}/{}/{}", self.character, self.check, self.layer)
        }
    }
}

struct CharacterInput {
    id: String,
    dir: PathBuf,
    manifest: Value,
}

pub fn run(args: &[String]) -> Result<()> {
    let mut only: Option<String> = None;
    let mut json_path: Option<PathBuf> = None;
    let mut strict = false;
    let mut update_baseline = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--strict" => strict = true,
            "--update-baseline" => update_baseline = true,
            "--character" => {
                index += 1;
                only = Some(
                    args.get(index)
                        .cloned()
                        .context("--character にキャラクターIDが必要です")?,
                );
            }
            "--json" => {
                index += 1;
                json_path = Some(PathBuf::from(
                    args.get(index).context("--json に出力先が必要です")?,
                ));
            }
            other => bail!("未対応のオプションです: {other}"),
        }
        index += 1;
    }
    if update_baseline && only.is_some() {
        bail!("--update-baseline は全キャラクターを対象にするため --character と併用できません");
    }

    let root = repo_root();
    let characters = collect_characters(&root.join("characters"), only.as_deref())?;
    if characters.is_empty() {
        bail!("検査対象の通常2Dキャラクターがありません");
    }

    let mut findings = Vec::new();
    let mut metrics = BTreeMap::new();
    for character in &characters {
        let (character_findings, character_metrics) = inspect(character)?;
        findings.extend(character_findings);
        metrics.insert(character.id.clone(), character_metrics);
    }

    let baseline_path = root.join(BASELINE_FILE);
    if update_baseline {
        let baseline = read_baseline(&baseline_path)?;
        if findings
            .iter()
            .any(|finding| !baseline.contains(&finding.key()))
        {
            bail!("新しい指摘はベースラインへ追加できません。素材を修正してから再検査してください");
        }
        write_baseline(&baseline_path, &findings)?;
        log(
            "character-quality",
            format!(
                "ベースラインへ{}件を記録しました: {}",
                findings.len(),
                baseline_path.display()
            ),
        );
        return Ok(());
    }

    let mut baseline = read_baseline(&baseline_path)?;
    if let Some(id) = only.as_deref() {
        baseline = narrow_baseline(&baseline, id);
    }
    let (fresh, known): (Vec<_>, Vec<_>) = findings
        .iter()
        .partition(|finding| !baseline.contains(&finding.key()));
    let resolved = baseline
        .iter()
        .filter(|key| !findings.iter().any(|finding| &finding.key() == *key))
        .cloned()
        .collect::<Vec<_>>();

    report(&characters, &metrics, &fresh, &known, &resolved);

    if let Some(path) = json_path {
        write_report(&path, &findings, &metrics, &baseline)?;
        log("character-quality", format!("検査結果: {}", path.display()));
    }

    let blocking = fresh
        .iter()
        .filter(|finding| finding.severity == Severity::Fail)
        .count();
    if blocking > 0 {
        bail!("ベースラインに無い不合格が{blocking}件あります");
    }
    if strict && !findings.is_empty() {
        bail!(
            "--strict のため、既知を含む指摘{}件を不合格として扱います",
            findings.len()
        );
    }
    Ok(())
}

/// 通常2Dかつ `motionProfile=full` のキャラクターだけを対象にする。
/// Live2Dは別管理で、9レイヤーの制作正本を持たない。
fn collect_characters(src: &Path, only: Option<&str>) -> Result<Vec<CharacterInput>> {
    if !src.is_dir() {
        bail!("キャラクターの正本が見つかりません: {}", src.display());
    }
    let mut dirs = fs::read_dir(src)?
        .collect::<std::io::Result<Vec<_>>>()?
        .into_iter()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir() && path.join("character.json").is_file())
        .collect::<Vec<_>>();
    dirs.sort();

    let mut characters = Vec::new();
    for dir in dirs {
        let Some(id) = dir.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if only.is_some_and(|wanted| wanted != id) {
            continue;
        }
        let manifest: Value =
            serde_json::from_str(&fs::read_to_string(dir.join("character.json"))?)
                .with_context(|| format!("{id} のcharacter.jsonを解析できません"))?;
        if !uses_layer_sources(&manifest) {
            continue;
        }
        characters.push(CharacterInput {
            id: id.to_string(),
            dir,
            manifest,
        });
    }
    if only.is_some() && characters.is_empty() {
        bail!("{} は通常2Dの検査対象ではありません", only.unwrap_or(""));
    }
    Ok(characters)
}

fn uses_layer_sources(manifest: &Value) -> bool {
    // 制作正本の欠落を対象外として隠さないため、ファイルの有無で判定しない。
    manifest.get("live2d").is_none()
        && manifest.get("motionProfile").and_then(Value::as_str) == Some("full")
        && (manifest.get("runtimeLayers").is_some() || manifest.get("meshRig").is_none())
}

/// 基準画像から一度だけ作る、レイヤー共通の判定材料。
struct BaseAnalysis {
    /// 不透明画素の範囲（左上・右下を含む）。
    content: (u32, u32, u32, u32),
    /// 不透明画素の総数。面積比の分母。
    subject: u64,
    /// 顔帯の下端 y。これより下は「顔以外」とみなす。
    face_bottom: u32,
    /// 輪郭（アンチエイリアス縁と内部線）かどうか。
    contour: Vec<bool>,
}

fn analyze_base(base: &RgbaImage) -> Option<BaseAnalysis> {
    let (width, height) = base.dimensions();
    let raw = base.as_raw();
    let pixels = (width * height) as usize;

    // 輝度はRGBの平均だが、除算を避けるため合計のまま持つ。しきい値は後で6倍して合わせる。
    let mut luminance = vec![0_i32; pixels];
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0_u32, 0_u32);
    let mut subject = 0_u64;
    for (index, value) in luminance.iter_mut().enumerate() {
        let offset = index * 4;
        *value = i32::from(raw[offset]) + i32::from(raw[offset + 1]) + i32::from(raw[offset + 2]);
        if raw[offset + 3] == 0 {
            continue;
        }
        subject += 1;
        let x = (index as u32) % width;
        let y = (index as u32) / width;
        x0 = x0.min(x);
        y0 = y0.min(y);
        x1 = x1.max(x);
        y1 = y1.max(y);
    }
    if x0 == u32::MAX {
        return None;
    }
    let face_bottom = y0 + (f64::from(y1 - y0 + 1) * FACE_BAND_RATIO) as u32;

    // 中心差分（端は片側差分）を2倍スケールで持ち、平均輝度換算のしきい値と比較する。
    // |dx/6| + |dy/6| > CONTOUR_GRADIENT  <=>  |dx| + |dy| > CONTOUR_GRADIENT * 6
    let limit = (CONTOUR_GRADIENT * 6.0) as i32;
    let mut contour = vec![false; pixels];
    // 幅または高さが1画素だと片側差分の参照先が隣の行へ回り込み、末尾で範囲外になる。
    // そもそも輪郭を論じられない画像なので、勾配判定ごと省く。
    if width < 2 || height < 2 {
        return Some(BaseAnalysis {
            content: (x0, y0, x1, y1),
            subject,
            face_bottom,
            contour,
        });
    }
    for y in 0..height {
        let row = (y * width) as usize;
        for x in 0..width {
            let index = row + x as usize;
            let alpha = raw[index * 4 + 3];
            if (1..=254).contains(&alpha) {
                contour[index] = true;
                continue;
            }
            let dx = if x == 0 {
                2 * (luminance[index + 1] - luminance[index])
            } else if x == width - 1 {
                2 * (luminance[index] - luminance[index - 1])
            } else {
                luminance[index + 1] - luminance[index - 1]
            };
            let dy = if y == 0 {
                2 * (luminance[index + width as usize] - luminance[index])
            } else if y == height - 1 {
                2 * (luminance[index] - luminance[index - width as usize])
            } else {
                luminance[index + width as usize] - luminance[index - width as usize]
            };
            contour[index] = dx.abs() + dy.abs() > limit;
        }
    }
    Some(BaseAnalysis {
        content: (x0, y0, x1, y1),
        subject,
        face_bottom,
        contour,
    })
}

fn inspect(character: &CharacterInput) -> Result<(Vec<Finding>, Value)> {
    let layers = character.dir.join("layers");
    let base_path = layers.join(format!("{BASE_LAYER}.png"));
    let base = load(&base_path)?;
    let mut findings = Vec::new();

    let Some(analysis) = analyze_base(&base) else {
        findings.push(Finding {
            severity: Severity::Fail,
            character: character.id.clone(),
            check: "被写体",
            layer: String::new(),
            message: format!("{} に不透明な画素がありません", base_path.display()),
        });
        return Ok((findings, json!({})));
    };

    findings.extend(check_canvas_margin(character, &base, analysis.content));
    findings.extend(check_rectangle_edges(character, &base, analysis.content));

    let mut layer_metrics = serde_json::Map::new();
    let mut half = None;
    let mut closed = None;
    for name in DIFFERENTIAL_LAYERS {
        let path = layers.join(format!("{name}.png"));
        if !path.is_file() {
            // 不足は assets.rs の同期時検証が止める。ここで二重に報告しない。
            continue;
        }
        let layer = load(&path)?;
        if layer.dimensions() != base.dimensions() {
            findings.push(Finding {
                severity: Severity::Fail,
                character: character.id.clone(),
                check: "寸法一致",
                layer: name.to_string(),
                message: format!(
                    "{name} が {}x{} で、base の {}x{} と違います。寸法が混ざると連番アニメが破綻します",
                    layer.width(),
                    layer.height(),
                    base.width(),
                    base.height()
                ),
            });
            continue;
        }
        let stats = compare(&base, &layer, &analysis);
        layer_metrics.insert(name.to_string(), stats.to_json());
        findings.extend(stats.findings(character, name));
        match name {
            "blink_half" => half = Some(layer),
            "blink_closed" => closed = Some(layer),
            _ => {}
        }
    }

    if let (Some(half), Some(closed)) = (half, closed) {
        findings.extend(check_half_blink(character, &half, &closed));
    }

    let mut metrics = display_metrics(character, &base, &analysis);
    findings.extend(check_generator_upscale(character, &analysis)?);
    findings.extend(check_native_scale(character, &metrics));
    metrics["layers"] = Value::Object(layer_metrics);
    Ok((findings, metrics))
}

fn load(path: &Path) -> Result<RgbaImage> {
    Ok(image::ImageReader::open(path)
        .with_context(|| format!("画像を開けません: {}", path.display()))?
        .with_guessed_format()?
        .decode()
        .with_context(|| format!("画像を復号できません: {}", path.display()))?
        .to_rgba8())
}

/// `analyze_base` は同じ走査のなかで内容枠を求めるため、この単体版はテスト専用にする。
#[cfg(test)]
fn content_box(image: &RgbaImage) -> Option<(u32, u32, u32, u32)> {
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0_u32, 0_u32);
    for (x, y, pixel) in image.enumerate_pixels() {
        if pixel.0[3] == 0 {
            continue;
        }
        x0 = x0.min(x);
        y0 = y0.min(y);
        x1 = x1.max(x);
        y1 = y1.max(y);
    }
    (x0 != u32::MAX).then_some((x0, y0, x1, y1))
}

/// 基準画像に対する1レイヤーの変化量。
#[derive(Default, Debug, PartialEq)]
struct LayerStats {
    changed: u64,
    /// 顔より下かつ輪郭でもない場所の変更。全身の描き直しはここに出る。
    outside_face: u64,
    /// 顔より下の輪郭の変更。レイヤーごとの再サンプリングはここに出る。
    contour_drift: u64,
    subject: u64,
}

impl LayerStats {
    fn ratio(&self, count: u64) -> f64 {
        if self.subject == 0 {
            return 0.0;
        }
        count as f64 / self.subject as f64
    }

    fn to_json(&self) -> Value {
        json!({
            "changedRatio": round4(self.ratio(self.changed)),
            "outsideFaceRatio": round4(self.ratio(self.outside_face)),
            "contourDriftRatio": round4(self.ratio(self.contour_drift)),
        })
    }

    fn findings(&self, character: &CharacterInput, name: &'static str) -> Vec<Finding> {
        if self.changed == 0 {
            return vec![Finding {
                severity: Severity::Warn,
                character: character.id.clone(),
                check: "差分なし",
                layer: name.to_string(),
                message: format!(
                    "{name} が base と一致します。レイヤーとして宣言されていても実際には動きません"
                ),
            }];
        }
        let mut findings = Vec::new();
        let outside = self.ratio(self.outside_face);
        if outside > OUTSIDE_FACE_LIMIT {
            findings.push(Finding {
                severity: Severity::Fail,
                character: character.id.clone(),
                check: "顔以外の書き換え",
                layer: name.to_string(),
                message: format!(
                    "{name} が顔より下の非輪郭部を被写体比{:.2}%書き換えています（上限{:.0}%）。\
                     全身の生成結果をそのまま差分にせず、顔の局所パッチへ切り出してください",
                    outside * 100.0,
                    OUTSIDE_FACE_LIMIT * 100.0
                ),
            });
        }
        let drift = self.ratio(self.contour_drift);
        if drift > CONTOUR_DRIFT_LIMIT {
            findings.push(Finding {
                severity: Severity::Warn,
                character: character.id.clone(),
                check: "輪郭ドリフト",
                layer: name.to_string(),
                message: format!(
                    "{name} で顔より下の輪郭が被写体比{:.2}%ずれています（上限{:.1}%）。\
                     レイヤーを個別に再サンプリングすると状態切替のたびに全身の輪郭が動きます",
                    drift * 100.0,
                    CONTOUR_DRIFT_LIMIT * 100.0
                ),
            });
        }
        findings
    }
}

fn compare(base: &RgbaImage, layer: &RgbaImage, analysis: &BaseAnalysis) -> LayerStats {
    let mut stats = LayerStats {
        subject: analysis.subject,
        ..LayerStats::default()
    };
    let width = base.width() as usize;
    let left = layer.as_raw();
    let right = base.as_raw();
    let face_bottom = analysis.face_bottom as usize;
    for index in 0..analysis.contour.len() {
        let offset = index * 4;
        // デバッグビルドでも1画素あたりの命令数を増やさないよう、反復子を使わず展開する。
        let delta = left[offset]
            .abs_diff(right[offset])
            .max(left[offset + 1].abs_diff(right[offset + 1]))
            .max(left[offset + 2].abs_diff(right[offset + 2]))
            .max(left[offset + 3].abs_diff(right[offset + 3]));
        if delta <= CHANGE_THRESHOLD {
            continue;
        }
        stats.changed += 1;
        if index / width <= face_bottom {
            continue;
        }
        if analysis.contour[index] {
            stats.contour_drift += 1;
        } else {
            stats.outside_face += 1;
        }
    }
    stats
}

/// 被写体がキャンバス端で切れていないか、四隅に背景が残っていないかを見る。
fn check_canvas_margin(
    character: &CharacterInput,
    base: &RgbaImage,
    content: (u32, u32, u32, u32),
) -> Vec<Finding> {
    let mut findings = Vec::new();
    let (x0, y0, x1, _) = content;
    let (width, height) = base.dimensions();

    // 立ち絵は足元が枠に接する構図があるため、下端の接触だけは許す。
    let touching = [(x0 == 0, "左"), (y0 == 0, "上"), (x1 == width - 1, "右")]
        .into_iter()
        .filter_map(|(hit, side)| hit.then_some(side))
        .collect::<Vec<_>>();
    if !touching.is_empty() {
        findings.push(Finding {
            severity: Severity::Fail,
            character: character.id.clone(),
            check: "余白",
            layer: String::new(),
            message: format!(
                "被写体がキャンバスの{}端で切れています。髪先や肩を切らずに余白を取ってください",
                touching.join("・")
            ),
        });
    }

    let opaque_corners = [
        (0, 0),
        (width - 1, 0),
        (0, height - 1),
        (width - 1, height - 1),
    ]
    .into_iter()
    .filter(|&(x, y)| base.get_pixel(x, y).0[3] != 0)
    .count();
    if opaque_corners > 0 {
        findings.push(Finding {
            severity: Severity::Fail,
            character: character.id.clone(),
            check: "透過",
            layer: String::new(),
            message: format!(
                "四隅のうち{opaque_corners}箇所が透明ではありません。クロマ背景か市松模様が焼き付いています"
            ),
        });
    }
    findings
}

/// クロマ抜きの取りこぼしや貼り付けた矩形パッチは、アルファ境界が長い直線になる。
/// 手描きの輪郭がここまで揃うことはまず無いので、直線の長さで検出する。
fn check_rectangle_edges(
    character: &CharacterInput,
    base: &RgbaImage,
    content: (u32, u32, u32, u32),
) -> Vec<Finding> {
    let (_, y0, _, y1) = content;
    let limit =
        ((f64::from(y1 - y0 + 1) * RECTANGLE_EDGE_RATIO) as u32).max(RECTANGLE_EDGE_MINIMUM_ROWS);
    let mut longest = 0_u32;
    let mut run = 0_u32;
    let mut previous: Option<(u32, u32)> = None;
    let raw = base.as_raw();
    let width = base.width();
    for y in y0..=y1 {
        let mut left = None;
        let mut right = 0;
        let row = (y * width) as usize;
        for x in 0..width {
            if raw[(row + x as usize) * 4 + 3] != 0 {
                left.get_or_insert(x);
                right = x;
            }
        }
        let current = left.map(|left| (left, right));
        run = if current.is_some() && current == previous {
            run + 1
        } else {
            1
        };
        longest = longest.max(run);
        previous = current;
    }
    if longest > limit {
        return vec![Finding {
            severity: Severity::Warn,
            character: character.id.clone(),
            check: "矩形",
            layer: String::new(),
            message: format!(
                "アルファ境界が{longest}行にわたって完全な直線です（上限{limit}行）。\
                 背景板か貼り付けた矩形が残っている可能性があります"
            ),
        }];
    }
    Vec::new()
}

/// 半目を「閉じ目の不透明度を下げただけ」で作っていないかを見る。
/// その作り方だとRGBが閉じ目と完全に一致し、アルファだけが違う画像になる。
fn check_half_blink(
    character: &CharacterInput,
    half: &RgbaImage,
    closed: &RgbaImage,
) -> Vec<Finding> {
    if half.dimensions() != closed.dimensions() {
        return Vec::new();
    }
    let mut alpha_only = 0_u64;
    let mut rgb_differs = 0_u64;
    for (left, right) in half.pixels().zip(closed.pixels()) {
        if left == right {
            continue;
        }
        if left.0[..3] == right.0[..3] {
            alpha_only += 1;
        } else {
            rgb_differs += 1;
        }
    }
    if alpha_only + rgb_differs == 0 {
        return vec![Finding {
            severity: Severity::Fail,
            character: character.id.clone(),
            check: "半目",
            layer: "blink_half".to_string(),
            message: "blink_half が blink_closed と完全に一致します。半目を専用に作画してください"
                .to_string(),
        }];
    }
    if rgb_differs == 0 {
        return vec![Finding {
            severity: Severity::Fail,
            character: character.id.clone(),
            check: "半目",
            layer: "blink_half".to_string(),
            message: format!(
                "blink_half が blink_closed と色は同一で、不透明度だけが{alpha_only}画素違います。\
                 閉じ目の透明度変更で半目を作らないでください"
            ),
        }];
    }
    Vec::new()
}

/// 既定構図で表示したときに原寸を割らないかを測る。
///
/// 立ち絵は `--char-frame-height` の枠へ `object-fit: contain` で収まり、そのうえで
/// `--char-zoom` 倍される。配信コピーは縦1800pxで頭打ちになるため、実際に使える
/// 画素数はそこで決まる。
fn display_metrics(character: &CharacterInput, base: &RgbaImage, analysis: &BaseAnalysis) -> Value {
    let (_, y0, _, y1) = analysis.content;
    let delivered_height = base.height().min(DELIVERY_IMAGE_MAX_HEIGHT);
    let max_zoom =
        f64::from(delivered_height) / (CHARACTER_FRAME_HEIGHT * ASSUMED_DEVICE_PIXEL_RATIO);
    json!({
        "sourceSize": [base.width(), base.height()],
        "contentHeight": y1 - y0 + 1,
        "deliveredHeight": delivered_height,
        "defaultZoom": default_zoom(&character.manifest),
        "maxNativeZoom": (max_zoom * 100.0).round() / 100.0,
        "nativeDetail": (native_detail(base, analysis.content) * 100.0).round() / 100.0,
    })
}

/// 利用者が構図を保存していないときに実際に適用される倍率。
/// `ui/renderer.js` の `applyCharacterFraming` と同じ優先順位で決める。
/// 最細スケールにどれだけ実情報が入っているかの指標。
///
/// 被写体を半分へ縮めて戻し、元画像との二乗平均平方根誤差を測る。引き伸ばして作った
/// 画像は最初から半分ぶんの情報しか持たないため往復してもほとんど変わらず、値が小さくなる。
/// 絵柄によって水準が変わる（写実は線画よりはるかに大きい）ので合否には使わず、
/// 同じ絵柄どうしを比べるための報告値として出す。
fn native_detail(base: &RgbaImage, content: (u32, u32, u32, u32)) -> f64 {
    let (x0, y0, x1, y1) = content;
    let (width, height) = (x1 - x0 + 1, y1 - y0 + 1);
    if width < 4 || height < 4 {
        return 0.0;
    }
    let crop = image::imageops::crop_imm(base, x0, y0, width, height).to_image();
    let small = image::imageops::resize(
        &crop,
        (width / 2).max(1),
        (height / 2).max(1),
        image::imageops::FilterType::Lanczos3,
    );
    let restored =
        image::imageops::resize(&small, width, height, image::imageops::FilterType::Lanczos3);

    let mut total = 0.0_f64;
    let mut counted = 0_u64;
    for (left, right) in crop.pixels().zip(restored.pixels()) {
        // 透明部は往復の影響を受けないため、被写体の内側だけを測る。
        if left.0[3] == 0 {
            continue;
        }
        for channel in 0..3 {
            let delta = f64::from(left.0[channel]) - f64::from(right.0[channel]);
            total += delta * delta;
        }
        counted += 3;
    }
    if counted == 0 {
        return 0.0;
    }
    (total / counted as f64).sqrt()
}

fn default_zoom(manifest: &Value) -> f64 {
    if let Some(zoom) = manifest
        .pointer("/defaultFraming/zoom")
        .and_then(Value::as_f64)
    {
        return zoom;
    }
    if manifest.get("bodyProportion").and_then(Value::as_str) == Some("compact") {
        1.55
    } else {
        2.05
    }
}

/// 生成器が記録している入力画像より大きい被写体を宣言していないかを、実画素で突き合わせる。
///
/// THA3のように入力解像度が固定された経路では、既存キャラクターと同じ縦幅へ揃えようとして
/// 引き伸ばし拡大が入り込みやすい。宣言だけを見ても分からないため、`generator.source` が
/// 指す原画の被写体と、完成レイヤーの被写体を直接比べる。統計ではなく実寸の比なので、
/// 絵柄によらず確実に判定できる。
fn check_generator_upscale(
    character: &CharacterInput,
    analysis: &BaseAnalysis,
) -> Result<Vec<Finding>> {
    let Some(relative) = character
        .manifest
        .pointer("/generator/source")
        .and_then(Value::as_str)
    else {
        return Ok(Vec::new());
    };
    let source_path = character.dir.join(relative);
    if !source_path.is_file() {
        return Ok(vec![Finding {
            severity: Severity::Fail,
            character: character.id.clone(),
            check: "生成元",
            layer: String::new(),
            message: format!(
                "generator.source が指す原画がありません: {}。生成元と突き合わせられないため、\
                 引き伸ばし拡大の有無を確認できません",
                source_path.display()
            ),
        }]);
    }
    let source = load(&source_path)?;
    let Some(source_analysis) = analyze_base(&source) else {
        return Ok(vec![Finding {
            severity: Severity::Fail,
            character: character.id.clone(),
            check: "生成元",
            layer: String::new(),
            message: format!("{} に不透明な画素がありません", source_path.display()),
        }]);
    };

    let (_, sy0, _, sy1) = source_analysis.content;
    let (_, y0, _, y1) = analysis.content;
    let source_height = f64::from(sy1 - sy0 + 1);
    let layer_height = f64::from(y1 - y0 + 1);
    let scale = layer_height / source_height;
    if scale <= GENERATOR_UPSCALE_TOLERANCE {
        return Ok(Vec::new());
    }
    Ok(vec![Finding {
        severity: Severity::Fail,
        character: character.id.clone(),
        check: "引き伸ばし拡大",
        layer: String::new(),
        message: format!(
            "完成レイヤーの被写体が縦{}pxで、生成元 {relative} の縦{}pxを{scale:.2}倍に引き伸ばしています。\
             拡大した画素は情報を増やさないため、生成元と同じ倍率までに収めるか、\
             高解像度の原画から作り直してください",
            layer_height as u32, source_height as u32
        ),
    }])
}

fn check_native_scale(character: &CharacterInput, metrics: &Value) -> Vec<Finding> {
    let max_zoom = metrics
        .get("maxNativeZoom")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let default_zoom = metrics
        .get("defaultZoom")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    if max_zoom >= default_zoom {
        return Vec::new();
    }
    let delivered = metrics
        .get("deliveredHeight")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let required =
        (default_zoom * CHARACTER_FRAME_HEIGHT * ASSUMED_DEVICE_PIXEL_RATIO).ceil() as u64;
    vec![Finding {
        severity: Severity::Warn,
        character: character.id.clone(),
        check: "原寸",
        layer: String::new(),
        message: format!(
            "既定構図の{default_zoom}倍では原寸を割ります（原寸を保てるのは{max_zoom}倍まで）。\
             配信される縦{delivered}pxに対し、デバイス比2の実機では縦{required}px必要です"
        ),
    }]
}

fn round4(value: f64) -> f64 {
    (value * 10_000.0).round() / 10_000.0
}

/// 1キャラクターだけ検査したときに、走査していない記録を解消済みと誤判定しないよう、
/// ベースラインを対象キャラクターぶんへ絞る。
fn narrow_baseline(baseline: &BTreeSet<String>, character: &str) -> BTreeSet<String> {
    let prefix = format!("{character}/");
    baseline
        .iter()
        .filter(|key| key.starts_with(&prefix))
        .cloned()
        .collect()
}

fn read_baseline(path: &Path) -> Result<BTreeSet<String>> {
    if !path.is_file() {
        return Ok(BTreeSet::new());
    }
    let value: Value = serde_json::from_str(&fs::read_to_string(path)?)
        .with_context(|| format!("{} を解析できません", path.display()))?;
    Ok(value
        .get("known")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default())
}

fn write_baseline(path: &Path, findings: &[Finding]) -> Result<()> {
    let known = findings
        .iter()
        .map(Finding::key)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let document = json!({
        "note": "cargo xtask character-quality の既知の指摘。未記録の種類・レイヤーを失敗にするための基準で、\
                 解消したら --update-baseline で減らす。増やすのは既存素材を作り直せない場合だけにする。",
        "known": known,
    });
    fs::write(path, serde_json::to_string_pretty(&document)? + "\n")?;
    Ok(())
}

fn report(
    characters: &[CharacterInput],
    metrics: &BTreeMap<String, Value>,
    fresh: &[&Finding],
    known: &[&Finding],
    resolved: &[String],
) {
    println!("キャラクター画素検査");
    for character in characters {
        let Some(metric) = metrics.get(&character.id) else {
            continue;
        };
        let number = |pointer: &str| {
            metric
                .pointer(pointer)
                .and_then(Value::as_f64)
                .unwrap_or(0.0)
        };
        println!(
            "  {:<20} 原画{}x{} / 配信縦{} / 既定{}倍 / 原寸維持{}倍まで / 実効解像度{}",
            character.id,
            number("/sourceSize/0"),
            number("/sourceSize/1"),
            number("/deliveredHeight"),
            number("/defaultZoom"),
            number("/maxNativeZoom"),
            number("/nativeDetail"),
        );
    }

    if !fresh.is_empty() {
        println!("\nベースラインに無い指摘");
        for finding in fresh {
            println!(
                "  [{}] {} / {}: {}",
                finding.severity.label(),
                finding.character,
                finding.check,
                finding.message
            );
        }
    }
    if !known.is_empty() {
        println!("\n既知の負債（ベースライン記録済み・{}件）", known.len());
        let mut counts: BTreeMap<(&str, &str), usize> = BTreeMap::new();
        for finding in known {
            *counts
                .entry((finding.character.as_str(), finding.check))
                .or_default() += 1;
        }
        for ((character, check), count) in counts {
            println!("  {character} / {check}: {count}件");
        }
    }
    if !resolved.is_empty() {
        println!(
            "\n解消済み（--update-baseline でベースラインから外せます・{}件）",
            resolved.len()
        );
        for key in resolved {
            println!("  {key}");
        }
    }
    if fresh.is_empty() && resolved.is_empty() {
        println!("\nベースラインからの変化はありません");
    }
}

fn write_report(
    path: &Path,
    findings: &[Finding],
    metrics: &BTreeMap<String, Value>,
    baseline: &BTreeSet<String>,
) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let report = json!({
        "version": 1,
        "metrics": metrics,
        "findings": findings
            .iter()
            .map(|finding| json!({
                "severity": finding.severity.label(),
                "character": finding.character,
                "check": finding.check,
                "layer": finding.layer,
                "message": finding.message,
                "known": baseline.contains(&finding.key()),
            }))
            .collect::<Vec<_>>(),
    });
    fs::write(path, serde_json::to_string_pretty(&report)? + "\n")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    #[test]
    fn 制作正本の対象はファイルの有無ではなく実行形式で判定する() {
        assert!(uses_layer_sources(&json!({"motionProfile":"full"})));
        assert!(uses_layer_sources(
            &json!({"motionProfile":"full", "meshRig":{}, "runtimeLayers":{}})
        ));
        assert!(!uses_layer_sources(
            &json!({"motionProfile":"full", "meshRig":{"rig":"rig2d/rig.json"}})
        ));
        assert!(!uses_layer_sources(
            &json!({"motionProfile":"full", "live2d":{}})
        ));
    }

    fn canvas(width: u32, height: u32) -> RgbaImage {
        RgbaImage::from_pixel(width, height, Rgba([0, 0, 0, 0]))
    }

    /// 中央に不透明な楕円を置いた、余白のある被写体。
    fn subject(width: u32, height: u32) -> RgbaImage {
        let mut image = canvas(width, height);
        let (cx, cy) = (f64::from(width) / 2.0, f64::from(height) / 2.0);
        let (rx, ry) = (f64::from(width) / 3.0, f64::from(height) / 3.0);
        for y in 0..height {
            for x in 0..width {
                let dx = (f64::from(x) - cx) / rx;
                let dy = (f64::from(y) - cy) / ry;
                if dx * dx + dy * dy <= 1.0 {
                    image.put_pixel(x, y, Rgba([200, 180, 170, 255]));
                }
            }
        }
        image
    }

    fn input(id: &str, manifest: Value) -> CharacterInput {
        CharacterInput {
            id: id.to_string(),
            dir: PathBuf::from("characters").join(id),
            manifest,
        }
    }

    #[test]
    fn content_box_ignores_transparent_padding() {
        let mut image = canvas(10, 10);
        image.put_pixel(3, 4, Rgba([1, 2, 3, 255]));
        image.put_pixel(6, 8, Rgba([1, 2, 3, 128]));
        assert_eq!(content_box(&image), Some((3, 4, 6, 8)));
        assert_eq!(content_box(&canvas(4, 4)), None);
    }

    #[test]
    fn a_local_face_patch_passes() {
        let base = subject(200, 400);
        let analysis = analyze_base(&base).unwrap();
        let mut layer = base.clone();
        // 顔帯の中だけを書き換える＝目や口の差分に相当する。
        for y in 140..160 {
            for x in 90..110 {
                layer.put_pixel(x, y, Rgba([10, 10, 10, 255]));
            }
        }
        let stats = compare(&base, &layer, &analysis);
        assert!(stats.changed > 0);
        assert_eq!(stats.outside_face, 0);
        assert!(stats
            .findings(&input("t", json!({})), "blink_closed")
            .is_empty());
    }

    #[test]
    fn a_whole_body_regeneration_fails() {
        let base = subject(200, 400);
        let analysis = analyze_base(&base).unwrap();
        let mut layer = base.clone();
        for pixel in layer.pixels_mut() {
            if pixel.0[3] != 0 {
                pixel.0[0] = pixel.0[0].saturating_add(40);
            }
        }
        let stats = compare(&base, &layer, &analysis);
        let findings = stats.findings(&input("t", json!({})), "expression_happy");
        assert!(findings.iter().any(
            |finding| finding.severity == Severity::Fail && finding.check == "顔以外の書き換え"
        ));
    }

    /// 輪郭だけがずれる（レイヤーごとの再サンプリング）ケースは、全身の描き直しと
    /// 区別して注意に落とす。
    fn shift_contour(base: &RgbaImage) -> RgbaImage {
        let mut layer = base.clone();
        for y in 1..base.height() - 1 {
            for x in 1..base.width() - 1 {
                let here = base.get_pixel(x, y).0[3];
                let right = base.get_pixel(x + 1, y).0[3];
                if here != right {
                    layer.put_pixel(x, y, *base.get_pixel(x + 1, y));
                }
            }
        }
        layer
    }

    #[test]
    fn contour_drift_is_a_warning_not_a_failure() {
        let base = subject(200, 400);
        let analysis = analyze_base(&base).unwrap();
        let stats = compare(&base, &shift_contour(&base), &analysis);
        let findings = stats.findings(&input("t", json!({})), "blink_half");
        assert!(findings
            .iter()
            .all(|finding| finding.severity == Severity::Warn));
        assert!(findings
            .iter()
            .any(|finding| finding.check == "輪郭ドリフト"));
    }

    #[test]
    fn a_layer_identical_to_base_is_reported() {
        let base = subject(200, 400);
        let analysis = analyze_base(&base).unwrap();
        let stats = compare(&base, &base.clone(), &analysis);
        let findings = stats.findings(&input("t", json!({})), "gaze_left");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Warn);
        assert_eq!(findings[0].check, "差分なし");
    }

    #[test]
    fn half_blink_must_not_be_a_transparency_change_of_the_closed_eye() {
        let closed = subject(100, 200);
        let mut half = closed.clone();
        for pixel in half.pixels_mut() {
            if pixel.0[3] == 255 {
                pixel.0[3] = 128;
            }
        }
        let findings = check_half_blink(&input("t", json!({})), &half, &closed);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Fail);

        let mut drawn = closed.clone();
        for y in 60..70 {
            for x in 40..60 {
                drawn.put_pixel(x, y, Rgba([20, 20, 30, 255]));
            }
        }
        assert!(check_half_blink(&input("t", json!({})), &drawn, &closed).is_empty());
    }

    #[test]
    fn canvas_margin_rejects_a_subject_cut_at_the_edge() {
        let mut image = canvas(50, 80);
        for y in 0..40 {
            for x in 0..20 {
                image.put_pixel(x, y, Rgba([9, 9, 9, 255]));
            }
        }
        let content = content_box(&image).unwrap();
        let findings = check_canvas_margin(&input("t", json!({})), &image, content);
        assert!(findings.iter().any(|finding| finding.check == "余白"));
        assert!(findings.iter().any(|finding| finding.check == "透過"));
    }

    /// しきい値は実測に基づく。滑らかな輪郭（完全な楕円でも被写体高さの0.139）を
    /// 巻き込まず、背景板（1.000）だけを拾う位置にあることを固定する。
    #[test]
    fn rectangle_edges_are_flagged_but_curves_are_not() {
        let mut plate = canvas(60, 200);
        for y in 20..180 {
            for x in 10..50 {
                plate.put_pixel(x, y, Rgba([0, 255, 0, 255]));
            }
        }
        let content = content_box(&plate).unwrap();
        assert!(!check_rectangle_edges(&input("t", json!({})), &plate, content).is_empty());

        let round = subject(200, 400);
        let content = content_box(&round).unwrap();
        assert!(check_rectangle_edges(&input("t", json!({})), &round, content).is_empty());
    }

    #[test]
    fn default_zoom_follows_the_renderer_priority() {
        assert_eq!(
            default_zoom(&json!({ "defaultFraming": { "zoom": 1.72 } })),
            1.72
        );
        assert_eq!(default_zoom(&json!({ "bodyProportion": "compact" })), 1.55);
        assert_eq!(default_zoom(&json!({ "bodyProportion": "standard" })), 2.05);
    }

    #[test]
    fn native_scale_warns_only_when_the_default_framing_magnifies() {
        let tall = subject(890, 1800);
        let analysis = analyze_base(&tall).unwrap();
        let character = input("t", json!({ "bodyProportion": "compact" }));
        let metrics = display_metrics(&character, &tall, &analysis);
        assert!(check_native_scale(&character, &metrics).is_empty());

        let short = subject(720, 1280);
        let analysis = analyze_base(&short).unwrap();
        let character = input("t", json!({ "bodyProportion": "standard" }));
        let metrics = display_metrics(&character, &short, &analysis);
        let findings = check_native_scale(&character, &metrics);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].check, "原寸");
    }

    /// 生成元と突き合わせる引き伸ばし検査。統計ではなく実寸の比で判定する。
    #[test]
    fn a_pack_larger_than_its_recorded_source_is_rejected() {
        let root = std::env::temp_dir().join(format!(
            "picoagent-generator-upscale-{}",
            std::process::id()
        ));
        let dir = root.join("sample");
        fs::create_dir_all(&dir).unwrap();

        // 生成元は小さな被写体、完成レイヤーはその5倍の被写体を持つ。
        subject(100, 200).save(dir.join("source.png")).unwrap();
        let layer = subject(500, 1000);
        let analysis = analyze_base(&layer).unwrap();

        let character = CharacterInput {
            id: "sample".to_string(),
            dir: dir.clone(),
            manifest: json!({ "generator": { "source": "source.png" } }),
        };
        let findings = check_generator_upscale(&character, &analysis).unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Fail);
        assert_eq!(findings[0].check, "引き伸ばし拡大");

        // 生成元を宣言していないパックは対象外。
        let plain = CharacterInput {
            id: "sample".to_string(),
            dir: dir.clone(),
            manifest: json!({}),
        };
        assert!(check_generator_upscale(&plain, &analysis)
            .unwrap()
            .is_empty());

        // 生成元と同じ大きさなら通る。
        let same = analyze_base(&subject(100, 200)).unwrap();
        assert!(check_generator_upscale(&character, &same)
            .unwrap()
            .is_empty());

        // 宣言した生成元が無ければ、確認できないこと自体を不合格にする。
        fs::remove_file(dir.join("source.png")).unwrap();
        let missing = check_generator_upscale(&character, &analysis).unwrap();
        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].check, "生成元");

        fs::remove_dir_all(&root).ok();
    }

    /// 引き伸ばした画像は最細スケールに情報が無いため、実効解像度が下がる。
    #[test]
    fn native_detail_drops_for_an_upscaled_image() {
        let original = subject(200, 400);
        let content = content_box(&original).unwrap();
        let small =
            image::imageops::resize(&original, 50, 100, image::imageops::FilterType::Lanczos3);
        let upscaled =
            image::imageops::resize(&small, 200, 400, image::imageops::FilterType::Lanczos3);

        let sharp = native_detail(&original, content);
        let blurred = native_detail(&upscaled, content_box(&upscaled).unwrap());
        assert!(
            blurred < sharp,
            "引き伸ばし後{blurred}が元画像{sharp}を下回らない"
        );
    }

    /// 検証ツール自体が不正な入力で落ちないこと。
    #[test]
    fn degenerate_images_do_not_panic() {
        let mut column = canvas(1, 8);
        column.put_pixel(0, 3, Rgba([10, 20, 30, 255]));
        let analysis = analyze_base(&column).expect("被写体がある");
        assert_eq!(analysis.subject, 1);
        assert!(analysis.contour.iter().all(|hit| !hit));
        assert_eq!(compare(&column, &column.clone(), &analysis).changed, 0);

        let mut row = canvas(8, 1);
        row.put_pixel(5, 0, Rgba([10, 20, 30, 255]));
        assert!(analyze_base(&row).is_some());

        assert!(analyze_base(&canvas(4, 4)).is_none());
    }

    #[test]
    fn narrowing_the_baseline_keeps_only_the_inspected_character() {
        let baseline = [
            "noa/半目/blink_half",
            "noa/差分なし/mouth_closed",
            "sena/差分なし/mouth_closed",
        ]
        .into_iter()
        .map(str::to_string)
        .collect::<BTreeSet<_>>();
        let narrowed = narrow_baseline(&baseline, "noa");
        assert_eq!(narrowed.len(), 2);
        assert!(narrowed.iter().all(|key| key.starts_with("noa/")));
        // 前方一致だけで別キャラを巻き込まないこと。
        assert!(narrow_baseline(&baseline, "no").is_empty());
    }

    #[test]
    fn baseline_keys_are_stable_against_message_changes() {
        let finding = Finding {
            severity: Severity::Fail,
            character: "sena".to_string(),
            check: "顔以外の書き換え",
            layer: "gaze_left".to_string(),
            message: "被写体比8.10%".to_string(),
        };
        assert_eq!(finding.key(), "sena/顔以外の書き換え/gaze_left");
        let renamed = Finding {
            message: "被写体比9.99%".to_string(),
            ..finding
        };
        assert_eq!(renamed.key(), "sena/顔以外の書き換え/gaze_left");
    }
}
