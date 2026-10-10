//! 検索結果の相対プロトコルと転送先パラメーターを一箇所で解決する。
pub fn direct_url(href: &str) -> String {
    let absolute = if href.starts_with("//") {
        format!("https:{href}")
    } else {
        href.to_string()
    };
    if let Ok(parsed) = url::Url::parse(&absolute) {
        if let Some((_, destination)) = parsed.query_pairs().find(|(key, _)| key == "uddg") {
            return destination.into_owned();
        }
    }
    absolute
}

#[cfg(test)]
mod tests {
    #[test]
    fn 検索リンクの形式と日本語転送先を保持する() {
        for (input, expected) in [
            ("//example.org/a", "https://example.org/a"),
            (
                "https://example.org/?uddg=https%3A%2F%2Fexample.jp%2F%E6%97%A5",
                "https://example.jp/日",
            ),
            ("/relative", "/relative"),
            ("not a url", "not a url"),
            ("https://example.org/?q=a", "https://example.org/?q=a"),
        ] {
            assert_eq!(super::direct_url(input), expected);
        }
    }
}
