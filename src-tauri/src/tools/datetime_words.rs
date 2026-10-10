//! 発話内の日数と相対時刻の整数を、欠損と不正を区別して読む。

fn integer_suffix(prefix: &str) -> Result<Option<i64>, String> {
    let start = prefix
        .char_indices()
        .rev()
        .find(|(_, ch)| !ch.is_ascii_digit())
        .map_or(0, |(index, ch)| index + ch.len_utf8());
    let digits = &prefix[start..];
    if digits.is_empty() {
        return Ok(None);
    }
    digits
        .parse()
        .map(Some)
        .map_err(|_| "発話の整数が扱える範囲を超えています".to_string())
}

pub fn number_before(text: &str, marker: &str) -> Result<Option<i64>, String> {
    match text.find(marker) {
        Some(index) => integer_suffix(&text[..index]),
        None => Ok(None),
    }
}

pub fn parse_days(text: &str) -> Result<Option<i64>, String> {
    for (index, ch) in text.char_indices() {
        let month = matches!(ch, 'ヶ' | 'か' | '月');
        if !month && ch != '日' {
            continue;
        }
        if let Some(number) = integer_suffix(&text[..index])? {
            let days = if month {
                number.checked_mul(30).ok_or("月数を日数へ換算できません")?
            } else {
                number
            };
            return Ok(Some(days.clamp(2, 365)));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    #[test]
    fn 発話の数値は欠損と上限と換算overflowを区別する() {
        for (text, expected) in [
            ("過去3ヶ月", Some(90)),
            ("2か月", Some(60)),
            ("1日", Some(2)),
            ("400日", Some(365)),
            ("日数なし", None),
        ] {
            assert_eq!(super::parse_days(text).unwrap(), expected);
        }
        for (text, marker, expected) in [
            ("仕事を25分", "分", Some(25)),
            ("日本語の分", "分", None),
            ("3時間後", "時間後", Some(3)),
            ("25 分", "分", None),
        ] {
            assert_eq!(super::number_before(text, marker).unwrap(), expected);
        }
        for text in ["9999999999999999999999日", "9223372036854775807ヶ月"] {
            assert!(super::parse_days(text).is_err());
        }
        assert!(super::number_before("9999999999999999999999分", "分").is_err());
    }
}
