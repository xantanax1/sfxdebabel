use std::collections::HashMap;

use async_trait::async_trait;
use serde::Deserialize;

use crate::error::AppError;
use crate::providers::{ProviderPage, ProviderSearchParams, RawSfxCandidate, SfxProvider};

pub struct FreesoundProvider {
    api_key: String,
}

impl FreesoundProvider {
    pub fn new(api_key: String) -> Self {
        Self { api_key }
    }
}

#[derive(Debug, Deserialize)]
struct FreesoundResponse {
    next: Option<String>,
    results: Vec<FreesoundSound>,
}

#[derive(Debug, Deserialize)]
struct FreesoundSound {
    id: u64,
    name: String,
    duration: f32,
    username: String,
    license: String,
    url: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    previews: HashMap<String, String>,
}

fn select_preview_url(previews: &HashMap<String, String>) -> Option<String> {
    const PRIORITY: [&str; 4] = [
        "preview-lq-ogg",
        "preview-lq-mp3",
        "preview-hq-mp3",
        "preview-hq-ogg",
    ];
    for key in PRIORITY {
        if let Some(url) = previews.get(key) {
            if !url.trim().is_empty() {
                return Some(url.clone());
            }
        }
    }
    previews.values().find(|u| !u.trim().is_empty()).cloned()
}

#[async_trait]
impl SfxProvider for FreesoundProvider {
    fn name(&self) -> &'static str {
        "Freesound"
    }

    async fn search(
        &self,
        client: &reqwest::Client,
        params: &ProviderSearchParams,
    ) -> Result<ProviderPage, AppError> {
        let key = self.api_key.trim();
        if key.is_empty() {
            return Err(AppError::MissingApiKey);
        }

        let min_d = params.min_duration.max(0.05);
        let max_d = params.max_duration.max(min_d + 0.1);
        let duration_filter = format!("duration:[{min_d:.2} TO {max_d:.2}]");
        let page_size = params.page_size.clamp(1, 50).to_string();
        let page = params.page.max(1).to_string();

        let resp = client
            .get("https://freesound.org/apiv2/search/text/")
            .header("Authorization", format!("Token {key}"))
            .query(&[
                ("query", params.query.trim()),
                ("filter", duration_filter.as_str()),
                (
                    "fields",
                    "id,name,duration,username,license,previews,tags,url",
                ),
                ("page", page.as_str()),
                ("page_size", page_size.as_str()),
            ])
            .send()
            .await?;

        let status = resp.status();
        if status.as_u16() == 401 || status.as_u16() == 403 {
            return Err(AppError::Provider(
                "Неверный или недействительный API-ключ Freesound (HTTP 401/403)".to_string(),
            ));
        }
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Provider(format!(
                "Freesound API вернул статус {status}: {body}"
            )));
        }

        let parsed: FreesoundResponse = resp.json().await?;
        let has_next = parsed.next.is_some();
        let mut out = Vec::with_capacity(parsed.results.len());

        for s in parsed.results {
            let Some(preview_url) = select_preview_url(&s.previews) else {
                continue;
            };
            out.push(RawSfxCandidate {
                id: format!("fs_{}", s.id),
                provider: "Freesound".to_string(),
                title: s.name,
                author: s.username,
                license: s.license,
                approx_duration: Some(s.duration),
                preview_url,
                source_url: s.url,
                tags: s.tags.into_iter().take(6).collect(),
            });
        }

        Ok(ProviderPage {
            candidates: out,
            has_next,
        })
    }
}
