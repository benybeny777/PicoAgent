// skills/artifact_pipeline/mod.rs - 調査結果から実際のOffice/PDF/HTML成果物を生成する永続パイプライン。

use std::io::{Cursor, Write};

use async_trait::async_trait;
use chrono::Local;
use serde_json::{json, Value};

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::permissions::SafePath;

pub struct ArtifactPipelineTool;

/// XMLが禁じる制御文字を落としつつ、実体参照へ置き換える。
/// 成果物1行ごとに呼ぶので、収集してから5回置換し直すのではなく1回の走査で組み立てる。
fn xml_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\u{9}' | '\u{A}' | '\u{D}' => out.push(ch),
            _ if ch >= '\u{20}' => out.push(ch),
            _ => {}
        }
    }
    out
}

fn html_escape(text: &str) -> String {
    xml_escape(text)
}

fn slug(text: &str) -> String {
    let value: String = text
        .chars()
        .take(40)
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let compact = value
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if compact.is_empty() {
        "project".to_string()
    } else {
        compact
    }
}

fn zip_bytes(parts: &[(&str, String)]) -> Result<Vec<u8>, String> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for (name, content) in parts {
        writer
            .start_file(*name, options)
            .map_err(|error| format!("Officeファイルの構築に失敗しました: {error}"))?;
        writer
            .write_all(content.as_bytes())
            .map_err(|error| format!("Officeファイルの書き込みに失敗しました: {error}"))?;
    }
    writer
        .finish()
        .map(|cursor| cursor.into_inner())
        .map_err(|error| format!("Officeファイルを確定できません: {error}"))
}

fn docx_bytes(title: &str, body: &str) -> Result<Vec<u8>, String> {
    let paragraphs = body
        .lines()
        .take(400)
        .map(|line| {
            format!(
                "<w:p><w:r><w:t xml:space=\"preserve\">{}</w:t></w:r></w:p>",
                xml_escape(line)
            )
        })
        .collect::<String>();
    zip_bytes(&[
        ("[Content_Types].xml", r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#.to_string()),
        ("_rels/.rels", r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#.to_string()),
        ("word/document.xml", format!(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:pPr><w:pStyle w:val="Title"/></w:pPr><w:r><w:t>{}</w:t></w:r></w:p>{}<w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440"/></w:sectPr></w:body></w:document>"#, xml_escape(title), paragraphs)),
    ])
}

fn xlsx_bytes(title: &str, body: &str) -> Result<Vec<u8>, String> {
    let mut rows = format!(
        "<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>{}</t></is></c></row>",
        xml_escape(title)
    );
    for (index, line) in body
        .lines()
        .filter(|line| !line.trim().is_empty())
        .take(300)
        .enumerate()
    {
        let row = index + 2;
        rows.push_str(&format!(
            "<row r=\"{row}\"><c r=\"A{row}\" t=\"inlineStr\"><is><t>{}</t></is></c></row>",
            xml_escape(line)
        ));
    }
    zip_bytes(&[
        ("[Content_Types].xml", r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>"#.to_string()),
        ("_rels/.rels", r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#.to_string()),
        ("xl/workbook.xml", r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="調査結果" sheetId="1" r:id="rId1"/></sheets></workbook>"#.to_string()),
        ("xl/_rels/workbook.xml.rels", r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#.to_string()),
        ("xl/worksheets/sheet1.xml", format!(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><cols><col min="1" max="1" width="80" customWidth="1"/></cols><sheetData>{rows}</sheetData></worksheet>"#)),
    ])
}

fn slide_xml(title: &str, body: &str) -> String {
    let title = xml_escape(title);
    let lines = body
        .lines()
        .filter(|line| !line.trim().is_empty())
        .take(12)
        .map(|line| {
            format!(
                "<a:p><a:r><a:rPr lang=\"ja-JP\" sz=\"2200\"/><a:t>{}</a:t></a:r></a:p>",
                xml_escape(line.trim_matches(['#', '-', '*', ' ']))
            )
        })
        .collect::<String>();
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/><a:chOff x="0" y="0"/><a:chExt cx="0" cy="0"/></a:xfrm></p:grpSpPr><p:sp><p:nvSpPr><p:cNvPr id="2" name="タイトル"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x="600000" y="350000"/><a:ext cx="10900000" cy="900000"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom><a:noFill/><a:ln><a:noFill/></a:ln></p:spPr><p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:rPr lang="ja-JP" sz="3200" b="1"/><a:t>{title}</a:t></a:r></a:p></p:txBody></p:sp><p:sp><p:nvSpPr><p:cNvPr id="3" name="本文"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x="850000" y="1500000"/><a:ext cx="10300000" cy="4700000"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom><a:noFill/><a:ln><a:noFill/></a:ln></p:spPr><p:txBody><a:bodyPr wrap="square"/><a:lstStyle/>{lines}</p:txBody></p:sp></p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sld>"#
    )
}

fn pptx_bytes(title: &str, body: &str) -> Result<Vec<u8>, String> {
    zip_bytes(&[
        ("[Content_Types].xml", r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/ppt/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/><Override PartName="/ppt/slides/slide1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slide+xml"/><Override PartName="/ppt/slideMasters/slideMaster1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slideMaster+xml"/><Override PartName="/ppt/slideLayouts/slideLayout1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml"/><Override PartName="/ppt/theme/theme1.xml" ContentType="application/vnd.openxmlformats-officedocument.theme+xml"/></Types>"#.to_string()),
        ("_rels/.rels", r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="ppt/presentation.xml"/></Relationships>"#.to_string()),
        ("ppt/presentation.xml", r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><p:presentation xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:sldMasterIdLst><p:sldMasterId id="2147483648" r:id="rId1"/></p:sldMasterIdLst><p:sldIdLst><p:sldId id="256" r:id="rId2"/></p:sldIdLst><p:sldSz cx="12192000" cy="6858000" type="screen16x9"/><p:notesSz cx="6858000" cy="9144000"/></p:presentation>"#.to_string()),
        ("ppt/_rels/presentation.xml.rels", r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster" Target="slideMasters/slideMaster1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide1.xml"/></Relationships>"#.to_string()),
        ("ppt/slides/slide1.xml", slide_xml(title, body)),
        ("ppt/slides/_rels/slide1.xml.rels", r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout" Target="../slideLayouts/slideLayout1.xml"/></Relationships>"#.to_string()),
        ("ppt/slideMasters/slideMaster1.xml", r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><p:sldMaster xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/><a:chOff x="0" y="0"/><a:chExt cx="0" cy="0"/></a:xfrm></p:grpSpPr></p:spTree></p:cSld><p:clrMap accent1="accent1" accent2="accent2" accent3="accent3" accent4="accent4" accent5="accent5" accent6="accent6" bg1="lt1" bg2="lt2" folHlink="folHlink" hlink="hlink" tx1="dk1" tx2="dk2"/><p:sldLayoutIdLst><p:sldLayoutId id="2147483649" r:id="rId1"/></p:sldLayoutIdLst><p:txStyles/></p:sldMaster>"#.to_string()),
        ("ppt/slideMasters/_rels/slideMaster1.xml.rels", r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout" Target="../slideLayouts/slideLayout1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme" Target="../theme/theme1.xml"/></Relationships>"#.to_string()),
        ("ppt/slideLayouts/slideLayout1.xml", r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><p:sldLayout xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" type="blank"><p:cSld name="空白"><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/><a:chOff x="0" y="0"/><a:chExt cx="0" cy="0"/></a:xfrm></p:grpSpPr></p:spTree></p:cSld></p:sldLayout>"#.to_string()),
        ("ppt/slideLayouts/_rels/slideLayout1.xml.rels", r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster" Target="../slideMasters/slideMaster1.xml"/></Relationships>"#.to_string()),
        ("ppt/theme/theme1.xml", r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" name="PicoAgent"><a:themeElements><a:clrScheme name="PicoAgent"><a:dk1><a:sysClr val="windowText" lastClr="000000"/></a:dk1><a:lt1><a:sysClr val="window" lastClr="FFFFFF"/></a:lt1><a:dk2><a:srgbClr val="1F2937"/></a:dk2><a:lt2><a:srgbClr val="F3F4F6"/></a:lt2><a:accent1><a:srgbClr val="2563EB"/></a:accent1><a:accent2><a:srgbClr val="059669"/></a:accent2><a:accent3><a:srgbClr val="D97706"/></a:accent3><a:accent4><a:srgbClr val="7C3AED"/></a:accent4><a:accent5><a:srgbClr val="DB2777"/></a:accent5><a:accent6><a:srgbClr val="0891B2"/></a:accent6><a:hlink><a:srgbClr val="0000FF"/></a:hlink><a:folHlink><a:srgbClr val="800080"/></a:folHlink></a:clrScheme><a:fontScheme name="PicoAgent"><a:majorFont><a:latin typeface="Aptos Display"/><a:ea typeface=""/><a:cs typeface=""/></a:majorFont><a:minorFont><a:latin typeface="Aptos"/><a:ea typeface=""/><a:cs typeface=""/></a:minorFont></a:fontScheme><a:fmtScheme name="PicoAgent"><a:fillStyleLst><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:fillStyleLst><a:lnStyleLst><a:ln w="9525"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:ln></a:lnStyleLst><a:effectStyleLst><a:effectStyle><a:effectLst/></a:effectStyle></a:effectStyleLst><a:bgFillStyleLst><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:bgFillStyleLst></a:fmtScheme></a:themeElements></a:theme>"#.to_string()),
    ])
}

/// このスキルが直接生成する成果物。並びは `build_artifacts()` の戻り値と対応させる。
const GENERATED_NAMES: [&str; 4] = ["report.html", "report.docx", "data.xlsx", "deck.pptx"];
/// 上の DOCX から OfficeCLI が変換して作る成果物。生成元が違うので別に持つ。
const CONVERTED_NAME: &str = "report.pdf";

fn build_artifacts(title: &str, body: &str) -> Result<Vec<Vec<u8>>, String> {
    let html = format!("<!doctype html><html lang=\"ja\"><meta charset=\"utf-8\"><title>{}</title><style>body{{font-family:system-ui;max-width:900px;margin:40px auto;line-height:1.7}}pre{{white-space:pre-wrap}}</style><h1>{}</h1><pre>{}</pre></html>", html_escape(title), html_escape(title), html_escape(body));
    Ok(vec![
        html.into_bytes(),
        docx_bytes(title, body)?,
        xlsx_bytes(title, body)?,
        pptx_bytes(title, body)?,
    ])
}

/// 成果物を生成して書き出す。
///
/// zip 圧縮と数MBのファイル出力は、そのまま `await` 中のタスクで回すと非同期ワーカーの
/// スレッドを1本占有する。許可判定（`cfg` が要る）だけ先に済ませ、重い生成と書き込みは
/// ブロッキング用スレッドへ渡す（`ai_drive` の読み書きと同じ扱いに揃える）。
async fn write_artifacts(
    cfg: &Config,
    dir: &SafePath,
    title: &str,
    body: &str,
) -> Result<Vec<SafePath>, String> {
    crate::permissions::fs::create_dir_all(dir)?;
    let paths = GENERATED_NAMES
        .iter()
        .map(|name| dir.join(cfg, name))
        .collect::<Result<Vec<SafePath>, String>>()?;
    let targets = paths.clone();
    let (title, body) = (title.to_string(), body.to_string());
    tokio::task::spawn_blocking(move || {
        for (path, bytes) in targets.iter().zip(build_artifacts(&title, &body)?) {
            crate::permissions::fs::write(path, bytes)?;
        }
        Ok(())
    })
    .await
    .map_err(|error| format!("成果物生成タスクが失敗しました: {error}"))?
    .map(|()| paths)
}

/// 出力先が「このジョブが作ったフォルダ」であることを所有者マーカーで確かめる。
/// 再起動をまたぐ再開でも、利用者の既存フォルダを上書きしないための境界。
fn ensure_owned_output(cfg: &Config, dir: &SafePath, pipeline_id: &str) -> Result<(), String> {
    let marker = dir.join(cfg, ".picoagent-artifact-job")?;
    if dir.exists() {
        let owner = crate::permissions::fs::read_to_string(&marker).map_err(|_| {
            format!(
                "既存フォルダは成果物ジョブ専用と確認できないため上書きしません: {}",
                dir.display()
            )
        })?;
        if owner.trim() != pipeline_id {
            return Err(format!(
                "別の成果物ジョブが所有するフォルダは上書きしません: {}",
                dir.display()
            ));
        }
        return Ok(());
    }
    crate::permissions::fs::create_dir_all(dir)?;
    // `permissions::fs::write()` ではなく `store::write_atomic()` を通す例外。
    // 所有者マーカーは上書き可否の判定そのものなので、書き込み途中で落ちて
    // 中身が空のマーカーが残ると、次の実行が別ジョブのフォルダを自分の物と誤認する。
    crate::store::write_atomic(marker.as_path(), pipeline_id.as_bytes())
        .map_err(|error| format!("成果物フォルダの所有情報を保存できません: {error:#}"))
}

async fn execute(cfg: &Config, args: &Value, job_id: &str) -> String {
    let topic = args
        .get("topic")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    // 出力先とジョブ所有者IDは登録時（run）に必ず埋める。ここで既定値を作り直すと、
    // 決め方が2箇所へ分かれるうえ、所有者IDが空のまま既存フォルダの上書き検査を素通りする。
    let requested = args
        .get("output_dir")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let pipeline_id = args
        .get("pipeline_id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if requested.is_empty() || pipeline_id.is_empty() {
        return "エラー: 永続ジョブの引数に出力先または所有者IDがありません。ジョブを登録し直してください。".to_string();
    }
    let dir = match crate::permissions::resolve(cfg, requested) {
        Ok(value) => value,
        Err(error) => return format!("エラー: {error}"),
    };
    if let Err(error) = ensure_owned_output(cfg, &dir, pipeline_id) {
        return format!("エラー: {error}");
    }
    let report_path = match dir.join(cfg, "research.md") {
        Ok(value) => value,
        Err(error) => return format!("エラー: {error}"),
    };
    let mut body = args
        .get("source_text")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if crate::durable_jobs::is_cancelled(job_id) {
        return "エラー: ジョブはキャンセルされました。".to_string();
    }
    if args
        .get("research")
        .and_then(Value::as_bool)
        .unwrap_or(true)
    {
        let result = crate::tools::dispatch_final(cfg, "research_report", json!({"topic":topic,"save":true,"artifact_bundle":true,"save_path":report_path.display().to_string()})).await;
        if result.starts_with("エラー:") {
            return result;
        }
        body = match crate::permissions::fs::read_to_string(&report_path) {
            Ok(value) => value,
            Err(error) => return format!("エラー: 調査レポートを再読込できません: {error}"),
        };
    }
    if body.trim().is_empty() {
        return "エラー: source_text が空です。research=trueにするか本文を指定してください。"
            .to_string();
    }
    if crate::durable_jobs::is_cancelled(job_id) {
        return "エラー: ジョブはキャンセルされました。".to_string();
    }
    let paths = match write_artifacts(cfg, &dir, topic, &body).await {
        Ok(value) => value,
        Err(error) => return format!("エラー: 成果物生成に失敗しました: {error}"),
    };
    if crate::durable_jobs::is_cancelled(job_id) {
        return "エラー: ジョブはキャンセルされました。".to_string();
    }
    let Some(docx) = paths.iter().find(|path| path.extension_lossy() == "docx") else {
        return "エラー: PDF変換の元になるDOCXを生成できませんでした。".to_string();
    };
    let pdf_result = crate::tools::dispatch_final(
        cfg,
        "ms_office",
        json!({"action":"export_pdf","input_path":docx.display().to_string()}),
    )
    .await;
    if pdf_result.starts_with("エラー:") {
        return format!("エラー: HTML/DOCX/XLSX/PPTXは生成しましたが、PDF生成に失敗しました。\n{pdf_result}\n出力先: {}", dir.display());
    }
    let provider = args
        .get("cloud_provider")
        .and_then(Value::as_str)
        .unwrap_or("");
    if !provider.is_empty() {
        let folder_id = args
            .get("cloud_folder_id")
            .and_then(Value::as_str)
            .unwrap_or("");
        let ai_root = crate::permissions::primary_root(cfg).join("PicoAgent-AI-Drive");
        for name in GENERATED_NAMES.iter().copied().chain([CONVERTED_NAME]) {
            if crate::durable_jobs::is_cancelled(job_id) {
                return "エラー: ジョブはキャンセルされました。".to_string();
            }
            let generated = match dir.join(cfg, name) {
                Ok(path) => path,
                Err(error) => return format!("エラー: クラウド転送対象を解決できません: {error}"),
            };
            let relative_path = match generated.as_path().strip_prefix(&ai_root) {
                Ok(path) => path.to_string_lossy().to_string(),
                Err(_) => return "エラー: クラウドへ続けて送る成果物はPicoAgent-AI-Drive配下へ出力してください。".to_string(),
            };
            let transfer_key = format!("artifact:{pipeline_id}:{provider}:{folder_id}:{name}");
            let remote_name = format!("{}-{pipeline_id}-{name}", dir.file_name_lossy());
            if let Err(error) = crate::durable_jobs::enqueue_unique(
                "ai_drive",
                json!({"action":"upload_execute","provider":provider,"relative_path":relative_path,"folder_id":folder_id,"remote_name":remote_name,"idempotency_key":transfer_key.clone()}),
                format!("{provider}へアップロード: {name}"),
                transfer_key,
            ) {
                return format!(
                    "エラー: 成果物は生成しましたが、クラウド転送ジョブを登録できません: {error}"
                );
            }
        }
    }
    let listed = GENERATED_NAMES
        .iter()
        .copied()
        .chain([CONVERTED_NAME])
        .map(|name| format!("\n- {name}"))
        .collect::<String>();
    format!("成果物を生成しました: {}{listed}", dir.display())
}

#[async_trait]
impl Tool for ArtifactPipelineTool {
    fn name(&self) -> &'static str {
        "artifact_pipeline"
    }
    fn label(&self) -> &'static str {
        "成果物パイプライン"
    }
    fn intent_keywords(&self) -> &'static [&'static str] {
        &["pptx", "xlsx", "docx", "成果物", "資料一式"]
    }
    fn description(&self) -> String {
        "Web調査または指定本文から、編集可能なPPTX/XLSX/DOCX、HTML、PDFを実ファイルとして生成する。処理は再起動後も再開する。".to_string()
    }
    fn parameters(&self) -> Value {
        json!({"type":"object","properties":{
            "topic":{"type":"string"},"source_text":{"type":"string"},
            "research":{"type":"boolean","description":"Web調査を行う。既定true。"},
            "output_dir":{"type":"string","description":"許可ルート配下の出力先。省略時はPicoAgent-AI-Drive/projects配下。"},
            "cloud_provider":{"type":"string","enum":["google","onedrive","icloud"],"description":"生成後のアップロード先。iCloudはmacOSのiCloud Driveへ保存する。"},
            "cloud_folder_id":{"type":"string"},
            "confirm_cloud":{"type":"boolean","description":"クラウド送信内容と送信先を確認済みならtrue。"}
        },"required":["topic"]})
    }
    async fn run(&self, _cfg: &Config, mut args: Value) -> String {
        if let Some(job_id) = crate::durable_jobs::worker_job_id(&args).map(str::to_string) {
            return execute(_cfg, &args, &job_id).await;
        }
        let topic = args
            .get("topic")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        if topic.is_empty() {
            return "エラー: topic が指定されていません。".to_string();
        }
        if topic.chars().count() > 160 {
            return "エラー: topic は160文字以内で指定してください。".to_string();
        }
        if args
            .get("source_text")
            .and_then(Value::as_str)
            .is_some_and(|text| text.len() > 100_000)
        {
            return "エラー: source_text は100KB以内で指定してください。".to_string();
        }
        let provider = args
            .get("cloud_provider")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if !provider.is_empty() && !matches!(provider.as_str(), "google" | "onedrive" | "icloud") {
            return "エラー: cloud_provider は google、onedrive、icloud のいずれかを指定してください。".to_string();
        }
        if !provider.is_empty()
            && !args
                .get("confirm_cloud")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        {
            return format!("確認が必要です。生成するHTML/DOCX/XLSX/PPTX/PDFを {provider} へ送信します。内容と送信先を確認し、confirm_cloud=true で再実行してください。");
        }
        if let Some(object) = args.as_object_mut() {
            object.remove("confirm_cloud");
            let missing_output = object
                .get("output_dir")
                .and_then(Value::as_str)
                .is_none_or(|value| value.trim().is_empty());
            if missing_output {
                object.insert(
                    "output_dir".into(),
                    Value::String(format!(
                        "PicoAgent-AI-Drive/projects/{}-{}",
                        Local::now().format("%Y%m%d-%H%M%S%3f"),
                        slug(&topic)
                    )),
                );
            }
            object.insert(
                "pipeline_id".into(),
                Value::String(format!(
                    "pipeline-{}",
                    Local::now().timestamp_nanos_opt().unwrap_or_default()
                )),
            );
        }
        let output = args.get("output_dir").and_then(Value::as_str).unwrap_or("");
        let output_path = match crate::permissions::resolve(_cfg, output) {
            Ok(path) => path,
            Err(error) => return format!("エラー: {error}"),
        };
        if output_path.exists() {
            return format!(
                "エラー: 既存フォルダは上書きしません。新しいoutput_dirを指定してください: {}",
                output_path.display()
            );
        }
        if !provider.is_empty() {
            let ai_root = crate::permissions::primary_root(_cfg).join("PicoAgent-AI-Drive");
            if !output_path.as_path().starts_with(&ai_root) {
                return "エラー: クラウドへ続けて送る成果物はPicoAgent-AI-Drive配下へ出力してください。".to_string();
            }
        }
        crate::durable_jobs::enqueue("artifact_pipeline", args, format!("成果物生成: {topic}"))
            .map(|id| format!("成果物生成をジョブへ登録しました: {id}"))
            .unwrap_or_else(|error| format!("エラー: {error}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn office成果物は必須部品を持つ() {
        for (bytes, required) in [
            (docx_bytes("題", "本文").unwrap(), "word/document.xml"),
            (
                xlsx_bytes("題", "本文").unwrap(),
                "xl/worksheets/sheet1.xml",
            ),
            (pptx_bytes("題", "本文").unwrap(), "ppt/slides/slide1.xml"),
        ] {
            let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
            assert!(archive.by_name(required).is_ok(), "{required}");
        }
    }

    #[test]
    fn 生成物の名前と中身の並びが揃っている() {
        // 名前と生成順がずれると、report.docx の中身が xlsx になっても誰も気付かない。
        let built = build_artifacts("題", "本文").unwrap();
        assert_eq!(built.len(), GENERATED_NAMES.len());
        for (name, bytes) in GENERATED_NAMES.iter().zip(&built) {
            let office =
                name.ends_with(".docx") || name.ends_with(".xlsx") || name.ends_with(".pptx");
            assert_eq!(bytes.starts_with(b"PK\x03\x04"), office, "{name}");
        }
    }

    #[test]
    fn xmlで禁止された制御文字を成果物へ混ぜない() {
        assert_eq!(xml_escape("A\u{0}B\u{8}C\nD"), "ABC\nD");
    }
}

inventory::submit! { ToolReg { make: || Box::new(ArtifactPipelineTool) } }
