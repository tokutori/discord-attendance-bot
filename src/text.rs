use unicode_normalization::UnicodeNormalization;

pub fn japanese_sort_key(value: &str) -> String {
    value
        .nfkc()
        .flat_map(char::to_lowercase)
        .map(|character| match character {
            'ァ'..='ヶ' => char::from_u32(character as u32 - 0x60).unwrap_or(character),
            _ => character,
        })
        .filter(|character| !character.is_whitespace())
        .collect()
}

pub fn normalize_name_reading(value: &str) -> Option<String> {
    let normalized = japanese_sort_key(value);
    (!normalized.is_empty()
        && normalized
            .chars()
            .all(|character| matches!(character, 'ぁ'..='ゖ' | 'ー' | '・' | 'ゝ' | 'ゞ')))
    .then_some(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_katakana_and_width_for_gojuon_sorting() {
        assert_eq!(
            normalize_name_reading(" ヤマダ タロウ ").as_deref(),
            Some("やまだたろう")
        );
        assert_eq!(normalize_name_reading("ﾔﾏﾀﾞ").as_deref(), Some("やまだ"));
        assert_eq!(normalize_name_reading("山田"), None);
    }
}
