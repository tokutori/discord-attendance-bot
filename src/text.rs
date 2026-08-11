use unicode_normalization::UnicodeNormalization;

pub const MAX_NAME_READING_CHARS: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum NameReadingError {
    #[error("名前の読みは、ひらがなまたはカタカナで入力してください")]
    InvalidCharacters,
    #[error("名前の読みは正規化後に{MAX_NAME_READING_CHARS}文字以内で入力してください")]
    TooLong,
}

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

pub fn normalize_name_reading(value: &str) -> Result<String, NameReadingError> {
    // Compatibility normalization is intentionally limited to input that is already
    // kana-like. This accepts half-width katakana without compatibility-folding arbitrary
    // names, symbols, or identifiers into different text.
    if !value.chars().all(|character| {
        character.is_whitespace()
            || matches!(
                character,
                'ぁ'..='ゖ'
                    | '゙'
                    | '゚'
                    | 'ゝ'
                    | 'ゞ'
                    | 'ァ'..='ヺ'
                    | 'ヽ'
                    | 'ヾ'
                    | 'ー'
                    | '・'
                    | '｡'..='ﾟ'
            )
    }) {
        return Err(NameReadingError::InvalidCharacters);
    }

    let normalized = japanese_sort_key(value);
    if normalized.is_empty()
        || !normalized
            .chars()
            .all(|character| matches!(character, 'ぁ'..='ゖ' | 'ー' | '・' | 'ゝ' | 'ゞ'))
    {
        return Err(NameReadingError::InvalidCharacters);
    }
    if normalized.chars().count() > MAX_NAME_READING_CHARS {
        return Err(NameReadingError::TooLong);
    }
    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_katakana_and_width_for_gojuon_sorting() {
        assert_eq!(
            normalize_name_reading(" ヤマダ タロウ ").unwrap(),
            "やまだたろう"
        );
        assert_eq!(normalize_name_reading("ﾔﾏﾀﾞ").unwrap(), "やまだ");
        assert_eq!(
            normalize_name_reading("山田"),
            Err(NameReadingError::InvalidCharacters)
        );
    }

    #[test]
    fn rejects_compatibility_characters_outside_kana_before_nfkc() {
        assert_eq!(
            normalize_name_reading("㍿"),
            Err(NameReadingError::InvalidCharacters)
        );
    }

    #[test]
    fn validates_length_after_normalization() {
        assert_eq!(
            normalize_name_reading(&"ア".repeat(MAX_NAME_READING_CHARS))
                .unwrap()
                .chars()
                .count(),
            MAX_NAME_READING_CHARS
        );
        assert_eq!(
            normalize_name_reading(&"ア".repeat(MAX_NAME_READING_CHARS + 1)),
            Err(NameReadingError::TooLong)
        );
    }
}
