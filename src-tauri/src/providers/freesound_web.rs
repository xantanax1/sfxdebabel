use std::collections::HashSet;

use async_trait::async_trait;
use scraper::{Html, Selector};

use crate::error::AppError;
use crate::license::LicenseInfo;
use crate::providers::http::{freesound_web_gate, send_with_retry};
use crate::providers::{ProviderPage, ProviderSearchParams, RawSfxCandidate, SfxProvider};

/// Keyless public web search provider for Freesound.org.
/// Used seamlessly as continuation after Mixkit or when a niche query has zero Mixkit results.
#[derive(Default)]
pub struct FreesoundWebProvider;

impl FreesoundWebProvider {
    pub fn new() -> Self {
        Self
    }
}

pub fn parse_freesound_web_html(html: &str) -> (Vec<RawSfxCandidate>, bool) {
    let document = Html::parse_document(html);
    let player_sel = Selector::parse(".bw-player").expect("valid bw-player selector");
    let next_sel = Selector::parse("a[title='Next Page'], a[aria-label='Next Page']")
        .expect("valid next page selector");

    let has_next = document.select(&next_sel).next().is_some();
    let mut seen_ids = HashSet::new();
    let mut results = Vec::new();

    for player in document.select(&player_sel) {
        let attrs = player.value();
        let Some(sound_id) = attrs
            .attr("data-sound-id")
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            continue;
        };

        let id = format!("fsw_{sound_id}");
        if !seen_ids.insert(id.clone()) {
            continue;
        }

        let preview_url = attrs
            .attr("data-ogg")
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .or_else(|| {
                attrs
                    .attr("data-mp3")
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
            });
        let Some(preview_url) = preview_url else {
            continue;
        };

        let title = attrs
            .attr("data-title")
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("Freesound Sample")
            .to_string();

        let author = attrs
            .attr("data-username")
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("Freesound User")
            .to_string();

        let approx_duration = attrs
            .attr("data-duration")
            .and_then(|d| d.trim().parse::<f32>().ok());

        results.push(RawSfxCandidate {
            id,
            provider: "Freesound".to_string(),
            title,
            author,
            // The search listing does not expose the license; do not guess.
            license: LicenseInfo::unknown(),
            approx_duration,
            preview_url: preview_url.to_string(),
            source_url: format!("https://freesound.org/s/{sound_id}/"),
            tags: Vec::new(),
        });
    }

    (results, has_next)
}

/// Like [`parse_freesound_web_html`], but tells "genuinely empty" apart from "markup changed".
pub fn parse_freesound_web_checked(
    html: &str,
) -> Result<(Vec<RawSfxCandidate>, bool), AppError> {
    let (items, has_next) = parse_freesound_web_html(html);
    if !items.is_empty() {
        return Ok((items, has_next));
    }

    let layout_err = |detail: &str| AppError::ProviderLayoutChanged {
        provider: "Freesound".to_string(),
        detail: detail.to_string(),
    };

    let document = Html::parse_document(html);
    let player_sel = Selector::parse(".bw-player").expect("valid bw-player selector");
    if document.select(&player_sel).next().is_some() {
        return Err(layout_err(
            ".bw-player found but data-sound-id / data-ogg / data-mp3 attributes are missing",
        ));
    }
    // Freesound renders an explicit "No results" notice for empty searches.
    if html.contains("No results") {
        return Ok((Vec::new(), false));
    }
    Err(layout_err(
        "no .bw-player elements and no 'No results' marker on the search page",
    ))
}

#[async_trait]
impl SfxProvider for FreesoundWebProvider {
    fn name(&self) -> &'static str {
        "Freesound"
    }

    async fn search(
        &self,
        client: &reqwest::Client,
        params: &ProviderSearchParams,
    ) -> Result<ProviderPage, AppError> {
        let query = if params.query.trim().is_empty() {
            "sfx"
        } else {
            params.query.trim()
        };
        let min_d = params.min_duration.max(0.05);
        let max_d = params.max_duration.max(min_d + 0.1);
        let filter = format!("duration:[{min_d:.2} TO {max_d:.2}]");
        let page = params.page.max(1).to_string();

        let resp = send_with_retry("Freesound", Some(freesound_web_gate()), || {
            client
                .get("https://freesound.org/search/")
                .header(
                    "Accept",
                    "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
                )
                .header("Accept-Language", "en-US,en;q=0.9")
                .query(&[
                    ("q", query),
                    ("f", filter.as_str()),
                    ("page", page.as_str()),
                ])
        })
        .await?;

        let status = resp.status();
        if !status.is_success() {
            return Err(AppError::Provider(format!(
                "Freesound Web вернул статус {status}"
            )));
        }

        let html = resp.text().await?;
        let (mut candidates, has_next) = parse_freesound_web_checked(&html)?;
        candidates.truncate(params.page_size.max(1));

        Ok(ProviderPage {
            candidates,
            has_next,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_freesound_web_html_and_pagination() {
        let sample = r#"
        <html>
          <body>
            <div
              class="bw-player"
              data-sound-id="25071"
              data-username="FreqMan"
              data-mp3="https://cdn.freesound.org/previews/25/25071_92661-lq.mp3"
              data-ogg="https://cdn.freesound.org/previews/25/25071_92661-lq.ogg"
              data-title="whoosh02.wav"
              data-duration="4.65067"
            ></div>
            <ul class="bw-pagination_container">
              <a href="/search/?q=whoosh&amp;page=2" title="Next Page" aria-label="Next Page">Next</a>
            </ul>
          </body>
        </html>
        "#;

        let (items, has_next) = parse_freesound_web_html(sample);
        assert!(has_next);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, "fsw_25071");
        assert_eq!(items[0].title, "whoosh02.wav");
        assert_eq!(items[0].author, "FreqMan");
        assert_eq!(
            items[0].preview_url,
            "https://cdn.freesound.org/previews/25/25071_92661-lq.ogg"
        );
        assert_eq!(items[0].approx_duration, Some(4.65067));
    }

    const FIXTURE_OK: &str = include_str!("../../tests/fixtures/freesound_web_whoosh.html");
    const FIXTURE_EMPTY: &str =
        include_str!("../../tests/fixtures/freesound_web_no_results.html");

    #[test]
    fn fixture_real_page_parses_players_and_next_page() {
        let (items, has_next) = parse_freesound_web_checked(FIXTURE_OK).expect("should parse");
        assert!(items.len() >= 10, "got {}", items.len());
        assert!(has_next, "fixture page 1 has a Next Page link");
        for it in &items {
            assert!(it.id.starts_with("fsw_"));
            assert!(it.preview_url.starts_with("http"));
            // The listing exposes no license: it must stay `unknown`, never guessed.
            assert_eq!(it.license.id, "unknown");
            assert_eq!(it.license.commercial_use_allowed, None);
        }
    }

    #[test]
    fn fixture_no_results_page_is_empty_not_error() {
        let (items, has_next) =
            parse_freesound_web_checked(FIXTURE_EMPTY).expect("empty is not an error");
        assert!(items.is_empty());
        assert!(!has_next);
    }

    #[test]
    fn renamed_player_class_raises_layout_changed() {
        let broken = FIXTURE_OK.replace("bw-player", "sound-player");
        let err = parse_freesound_web_checked(&broken).unwrap_err();
        assert!(matches!(err, AppError::ProviderLayoutChanged { .. }), "{err:?}");
        assert!(err.to_string().starts_with("PROVIDER_LAYOUT_CHANGED|Freesound|"));
    }

    #[test]
    fn renamed_data_attributes_raise_layout_changed() {
        let broken = FIXTURE_OK.replace("data-sound-id", "data-snd");
        let err = parse_freesound_web_checked(&broken).unwrap_err();
        assert!(matches!(err, AppError::ProviderLayoutChanged { .. }), "{err:?}");
    }
}
