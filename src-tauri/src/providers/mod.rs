pub mod freesound;
pub mod freesound_web;
pub mod mixkit;

use std::collections::HashSet;

use async_trait::async_trait;

use crate::error::AppError;

#[derive(Debug, Clone)]
pub struct RawSfxCandidate {
    pub id: String,
    pub provider: String,
    pub title: String,
    pub author: String,
    pub license: String,
    pub approx_duration: Option<f32>,
    pub preview_url: String,
    pub source_url: String,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ProviderPage {
    pub candidates: Vec<RawSfxCandidate>,
    pub has_next: bool,
}

#[derive(Debug, Clone)]
pub struct ProviderSearchParams {
    pub query: String,
    pub min_duration: f32,
    pub max_duration: f32,
    pub page: u32,
    pub page_size: usize,
}

#[async_trait]
pub trait SfxProvider: Send + Sync {
    fn name(&self) -> &'static str;

    async fn search(
        &self,
        client: &reqwest::Client,
        params: &ProviderSearchParams,
    ) -> Result<ProviderPage, AppError>;
}

const RU_TO_EN_SFX: &[(&str, &str)] = &[
    ("вуш", "whoosh"),
    ("свиш", "whoosh"),
    ("взмах", "whoosh"),
    ("пролет", "whoosh"),
    ("переход", "transition"),
    ("удар", "impact"),
    ("толчок", "punch"),
    ("хрясь", "hit"),
    ("шлепок", "slap"),
    ("хлопок", "clap"),
    ("клик", "click"),
    ("щелчок", "click"),
    ("кнопка", "button"),
    ("интерфейс", "ui"),
    ("меню", "menu"),
    ("уведомлен", "notification"),
    ("поп", "pop"),
    ("райзер", "riser"),
    ("подъем", "riser"),
    ("нараста", "riser"),
    ("саб", "sub"),
    ("дроп", "drop"),
    ("бас", "bass"),
    ("бум", "boom"),
    ("выстрел", "gunshot"),
    ("пистолет", "gun"),
    ("автомат", "rifle"),
    ("взрыв", "explosion"),
    ("огонь", "fire"),
    ("пламя", "fire"),
    ("дождь", "rain"),
    ("ливень", "rain"),
    ("ветер", "wind"),
    ("буря", "storm"),
    ("гром", "thunder"),
    ("молния", "thunder"),
    ("вода", "water"),
    ("всплеск", "splash"),
    ("капл", "water drop"),
    ("шаг", "footstep"),
    ("двер", "door"),
    ("скрип", "creak"),
    ("стекло", "glass"),
    ("разби", "smash"),
    ("лазер", "laser"),
    ("бластер", "laser"),
    ("меч", "sword"),
    ("клинок", "blade"),
    ("магия", "magic"),
    ("заклинан", "spell"),
    ("глитч", "glitch"),
    ("помех", "static"),
    ("сирена", "siren"),
    ("тревог", "alarm"),
    ("звонок", "bell"),
    ("колокол", "bell"),
    ("сердц", "heartbeat"),
    ("пульс", "heartbeat"),
    ("час", "clock"),
    ("тик", "tick"),
    ("металл", "metal"),
    ("желез", "metal"),
    ("бумаг", "paper"),
    ("страниц", "page"),
    ("клавиат", "keyboard"),
    ("печат", "typing"),
    ("камера", "camera"),
    ("затвор", "shutter"),
    ("машин", "car"),
    ("двигател", "engine"),
    ("мотор", "engine"),
    ("фон", "ambient"),
    ("амбиент", "ambient"),
    ("атмосфер", "ambience"),
    ("лес", "forest"),
    ("птиц", "birds"),
    ("смех", "laugh"),
    ("крик", "scream"),
    ("ошибк", "error"),
    ("успех", "success"),
    ("монет", "coin"),
    ("деньг", "cash"),
    ("касс", "cash register"),
];

/// Translates common Russian SFX queries into English search terms while keeping
/// ASCII words unchanged so Mixkit and Freesound return relevant sounds instead of falling back to homepages.
pub fn normalize_sfx_query(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let mut out_tokens: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    for token in trimmed
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
    {
        let lower = token.to_lowercase().replace('ё', "е");
        if lower.is_ascii() {
            if seen.insert(lower.clone()) {
                out_tokens.push(lower);
            }
            continue;
        }

        if let Some((_, mapped)) = RU_TO_EN_SFX
            .iter()
            .find(|(prefix, _)| lower.starts_with(prefix))
        {
            for en_tok in mapped.split_whitespace() {
                if seen.insert(en_tok.to_string()) {
                    out_tokens.push(en_tok.to_string());
                }
            }
        }
    }

    if out_tokens.is_empty() {
        // Preserve original text if no dictionary entry matched (Freesound supports multilingual tags).
        trimmed.to_string()
    } else {
        out_tokens.join(" ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translates_russian_sfx_terms_to_english() {
        assert_eq!(normalize_sfx_query("удар"), "impact");
        assert_eq!(normalize_sfx_query("сильный выстрел whoosh"), "gunshot whoosh");
        assert_eq!(normalize_sfx_query("саб дроп"), "sub drop");
        assert_eq!(normalize_sfx_query("ui click"), "ui click");
        assert_eq!(normalize_sfx_query("   "), "");
    }
}
