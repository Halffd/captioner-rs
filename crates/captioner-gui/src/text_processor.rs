use std::sync::OnceLock;
use tracing::warn;

static TAGGER: OnceLock<Option<lindera::Tagger>> = OnceLock::new();

fn contains_japanese(text: &str) -> bool {
    text.chars().any(|c| {
        let code = c as u32;
        (0x3040..=0x309F).contains(&code)
            || (0x30A0..=0x30FF).contains(&code)
            || (0x4E00..=0x9FFF).contains(&code)
    })
}

fn is_kanji(c: char) -> bool {
    let code = c as u32;
    (0x4E00..=0x9FFF).contains(&code)
}

fn get_tagger() -> Option<&'static lindera::Tagger> {
    TAGGER.get_or_init(|| {
        match lindera::Tagger::new(lindera::TokenizerConfig::default()) {
            Ok(t) => Some(t),
            Err(e) => {
                eprintln!(
                    "Note: Japanese text processing unavailable (lindera init failed: {})",
                    e
                );
                None
            }
        }
    }).as_ref()
}

pub fn process_text_with_furigana(text: &str) -> String {
    let tagger = match get_tagger() {
        Some(t) => t,
        None => return text.to_string(),
    };

    if !contains_japanese(text) {
        return text.to_string();
    }

    let tokens = match tagger.tokenize(text) {
        Ok(tokens) => tokens,
        Err(e) => {
            warn!("Furigana tokenization error: {}", e);
            return text.to_string();
        }
    };

    let mut result = String::new();
    for token in tokens {
        let surface = token.surface;
        if surface.is_empty() {
            continue;
        }

        if let Some(ref details) = token.detail {
            if details.len() > 7 && details[7] != "*" && details[7] != surface {
                let reading = &details[7];
                if surface.chars().any(is_kanji) {
                    result.push_str(surface);
                    result.push('(');
                    result.push_str(reading);
                    result.push(')');
                    continue;
                }
            }
        }

        result.push_str(surface);
    }

    result
}