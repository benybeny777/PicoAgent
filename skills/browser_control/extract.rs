// skills/browser_control/extract.rs - ページ内容の簡約と整形。
//
// 生の DOM をそのままモデルへ渡すとコンテキストが即座に溢れるため、ページ内で
// 「本文テキスト」と「操作できる要素の番号付き一覧」まで削ってから受け取る。
// モデルには CSS セレクタを書かせず、番号だけを扱わせる方針（小型ローカルモデルでも成立する）。

use serde_json::Value;

/// ページ内で実行する抽出スクリプト。返り値は JSON 文字列。
///
/// 要素の実体は `window.__picoAgentElements` に残す。読み取り専用の現段階では使わないが、
/// 操作（クリック・入力）を足すときに番号から同じ要素へ辿れるようにしておく。
pub const EXTRACT_JS: &str = r#"
(() => {
  const norm = (value) => (value || "").replace(/\s+/g, " ").trim();
  const isVisible = (el) => {
    const rect = el.getBoundingClientRect();
    if (rect.width <= 0 || rect.height <= 0) return false;
    const style = window.getComputedStyle(el);
    return style.visibility !== "hidden" && style.display !== "none" && style.opacity !== "0";
  };
  const selector = [
    "a[href]", "button", "input", "select", "textarea",
    "[role=button]", "[role=link]", "[role=textbox]", "[role=checkbox]", "[onclick]"
  ].join(", ");
  const nodes = [];
  const elements = [];
  for (const el of document.querySelectorAll(selector)) {
    if (elements.length >= 200) break;
    if (nodes.includes(el)) continue;
    if (!isVisible(el)) continue;
    if (el.disabled) continue;
    const tag = el.tagName.toLowerCase();
    let kind = tag;
    if (tag === "a") kind = "link";
    else if (tag === "input") kind = "input:" + (el.getAttribute("type") || "text");
    let label = norm(
      el.getAttribute("aria-label") || el.innerText || el.value ||
      el.getAttribute("placeholder") || el.getAttribute("title") || el.getAttribute("name") || ""
    );
    if (label.length > 80) label = label.slice(0, 80) + "…";
    if (!label && kind === "link") label = norm(el.getAttribute("href"));
    nodes.push(el);
    elements.push({ i: elements.length + 1, kind, label, href: el.getAttribute("href") || "" });
  }
  window.__picoAgentElements = nodes;
  const body = document.body ? document.body.innerText : "";
  // CDP 応答へ巨大な DOM 全文を複製しない。表示用の分割とは別に、ブラウザから
  // アプリへ渡す時点で上限を置いてメモリ枯渇を防ぐ。
  const text = body.replace(/[ \t ]+/g, " ").replace(/\n{3,}/g, "\n\n").trim().slice(0, 1000000);
  return JSON.stringify({ title: document.title || "", url: location.href, text, elements });
})()
"#;

pub struct Page {
    pub title: String,
    pub url: String,
    pub text: String,
    pub elements: Vec<Element>,
}

pub struct Element {
    pub index: u64,
    pub kind: String,
    pub label: String,
    pub href: String,
}

/// 抽出スクリプトの戻り値（JSON 文字列）を構造体へ変換する。
pub fn parse(value: &Value) -> Result<Page, String> {
    let raw = value
        .as_str()
        .ok_or_else(|| "エラー: ページの内容を取得できませんでした。".to_string())?;
    let parsed: Value = serde_json::from_str(raw)
        .map_err(|error| format!("エラー: ページ内容の解析に失敗しました: {error}"))?;
    let elements = parsed
        .get("elements")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|entry| {
                    Some(Element {
                        index: entry.get("i")?.as_u64()?,
                        kind: entry.get("kind")?.as_str()?.to_string(),
                        label: entry
                            .get("label")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        href: entry
                            .get("href")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(Page {
        title: parsed
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        url: parsed
            .get("url")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        text: parsed
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        elements,
    })
}

/// 文字数予算に収まるよう本文を切り出す。`offset` から始めて、続きがあれば案内を付ける。
///
/// 打ち切りではなく**続きを読める**形にしているのは、コンテキストの狭いローカルモデルでも
/// 長いページを数回に分けて読み切れるようにするため。
pub fn slice_text(text: &str, offset: usize, budget: usize) -> (String, Option<usize>) {
    let chars: Vec<char> = text.chars().collect();
    if offset >= chars.len() {
        return (String::new(), None);
    }
    let end = (offset + budget).min(chars.len());
    let body: String = chars[offset..end].iter().collect();
    let next = if end < chars.len() { Some(end) } else { None };
    (body, next)
}

/// 操作できる要素の一覧を、番号付きの短い行で整形する。
pub fn format_elements(elements: &[Element], limit: usize) -> String {
    if elements.is_empty() {
        return "（操作できる要素は見つかりませんでした）".to_string();
    }
    let mut out = String::new();
    for element in elements.iter().take(limit) {
        let label = if element.label.is_empty() {
            "（ラベルなし）"
        } else {
            element.label.as_str()
        };
        // リンクは飛び先も短く添える（どれを読みに行くかの判断材料になる）。
        let href = match element.href.as_str() {
            "" => String::new(),
            href if href.chars().count() > 60 => {
                format!(" -> {}…", href.chars().take(60).collect::<String>())
            }
            href => format!(" -> {href}"),
        };
        out.push_str(&format!(
            "  {} [{}] {}{}\n",
            element.index, element.kind, label, href
        ));
    }
    if elements.len() > limit {
        out.push_str(&format!("  …ほか {} 件\n", elements.len() - limit));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{format_elements, parse, slice_text, Element};
    use serde_json::json;

    #[test]
    fn 本文は予算ごとに区切って続きの位置を返す() {
        let text: String = "あ".repeat(10);
        let (body, next) = slice_text(&text, 0, 4);
        assert_eq!(body.chars().count(), 4);
        assert_eq!(next, Some(4));
        let (body, next) = slice_text(&text, 8, 4);
        assert_eq!(body.chars().count(), 2);
        assert_eq!(next, None, "末尾まで読んだら続きは無い");
    }

    #[test]
    fn 範囲外のオフセットは空を返す() {
        let (body, next) = slice_text("abc", 10, 100);
        assert!(body.is_empty());
        assert_eq!(next, None);
    }

    #[test]
    fn 抽出結果のjsonを構造体へ変換できる() {
        let raw = json!({
            "title": "テスト",
            "url": "https://example.com/",
            "text": "本文",
            "elements": [{ "i": 1, "kind": "link", "label": "次へ", "href": "/next" }]
        })
        .to_string();
        let page = parse(&json!(raw)).unwrap();
        assert_eq!(page.title, "テスト");
        assert_eq!(page.elements.len(), 1);
        assert_eq!(page.elements[0].label, "次へ");
    }

    #[test]
    fn 要素一覧は上限を超えたら残数を示す() {
        let elements: Vec<Element> = (1..=5)
            .map(|i| Element {
                index: i,
                kind: "link".to_string(),
                label: format!("項目{i}"),
                href: String::new(),
            })
            .collect();
        let formatted = format_elements(&elements, 2);
        assert!(formatted.contains("1 [link] 項目1"));
        assert!(formatted.contains("ほか 3 件"));
    }
}
