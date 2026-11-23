use std::collections::HashSet;

use async_trait::async_trait;
use scraper::{Html, Selector};

use crate::error::AppError;
use crate::license::LicenseInfo;
use crate::providers::http::{mixkit_gate, send_with_retry};
use crate::providers::{ProviderPage, ProviderSearchParams, RawSfxCandidate, SfxProvider};

#[derive(Default)]
pub struct MixkitProvider;

impl MixkitProvider {
    pub fn new() -> Self {
        Self
    }

    async fn fetch_html(&self, client: &reqwest::Client, url: &str) -> Result<String, AppError> {
        let resp = send_with_retry("Mixkit", Some(mixkit_gate()), || {
            client
                .get(url)
                .header(
                    "Accept",
                    "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
                )
                .header("Accept-Language", "en-US,en;q=0.9")
        })
        .await?;

        if !resp.status().is_success() {
            return Err(AppError::Provider(format!(
                "Mixkit вернул статус {}",
                resp.status()
            )));
        }
        Ok(resp.text().await?)
    }
}

impl MixkitProvider {
    /// Fetches and parses one page. Plain HTTP failures (404, 5xx, network) count as "no
    /// results here" so the fallback chain can continue; rate limiting aborts immediately and
    /// layout changes are remembered in `layout_err`.
    async fn fetch_and_parse(
        &self,
        client: &reqwest::Client,
        url: &str,
        layout_err: &mut Option<AppError>,
    ) -> Result<(Vec<RawSfxCandidate>, bool), AppError> {
        let html = match self.fetch_html(client, url).await {
            Ok(h) => h,
            Err(e @ AppError::RateLimited { .. }) => return Err(e),
            Err(_) => return Ok((Vec::new(), false)),
        };
        match parse_mixkit_checked(&html) {
            Ok(page) => Ok(page),
            Err(e @ AppError::ProviderLayoutChanged { .. }) => {
                layout_err.get_or_insert(e);
                Ok((Vec::new(), false))
            }
            Err(e) => Err(e),
        }
    }
}

fn tokenize_query(query: &str) -> Vec<String> {
    query
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_ascii_lowercase())
        .collect()
}

fn append_page_param(base: &str, page: u32) -> String {
    if page <= 1 {
        base.to_string()
    } else {
        format!("{base}?page={page}")
    }
}

pub fn parse_duration_str(raw: &str) -> Option<f32> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let parts: Vec<&str> = trimmed.split(':').collect();
    match parts.as_slice() {
        [secs] => secs.trim().parse::<f32>().ok(),
        [mins, secs] => {
            let m = mins.trim().parse::<f32>().ok()?;
            let s = secs.trim().parse::<f32>().ok()?;
            Some(m * 60.0 + s)
        }
        [hrs, mins, secs] => {
            let h = hrs.trim().parse::<f32>().ok()?;
            let m = mins.trim().parse::<f32>().ok()?;
            let s = secs.trim().parse::<f32>().ok()?;
            Some(h * 3600.0 + m * 60.0 + s)
        }
        _ => None,
    }
}

pub fn parse_mixkit_page(html: &str) -> (Vec<RawSfxCandidate>, bool) {
    let document = Html::parse_document(html);

    let item_sel = Selector::parse(".item-grid__item").expect("valid item selector");
    let player_sel =
        Selector::parse("[data-test-id='audio-player']").expect("valid player selector");
    let title_sel = Selector::parse(".item-grid-card__title").expect("valid title selector");
    let duration_sel = Selector::parse("[data-test-id='duration']").expect("valid duration selector");
    let tag_sel = Selector::parse(".meta-links__link").expect("valid tag selector");
    let next_sel =
        Selector::parse("link[rel='next'], a[rel='next'], .pagination__link--next")
            .expect("valid next selector");

    let has_next = document.select(&next_sel).next().is_some();

    let mut seen_ids = HashSet::new();
    let mut results = Vec::new();

    for card in document.select(&item_sel) {
        let Some(player) = card.select(&player_sel).next() else {
            continue;
        };
        let Some(preview_url) = player
            .value()
            .attr("data-audio-player-preview-url-value")
            .map(str::trim)
            .filter(|u| !u.is_empty())
        else {
            continue;
        };

        let item_id = player
            .value()
            .attr("data-audio-player-item-id-value")
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| format!("mk_{s}"))
            .unwrap_or_else(|| format!("mk_{}", results.len() + 1));

        if !seen_ids.insert(item_id.clone()) {
            continue;
        }

        let title = card
            .select(&title_sel)
            .next()
            .map(|el| el.text().collect::<String>().trim().to_string())
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| "Mixkit Sound".to_string());

        let approx_duration = card
            .select(&duration_sel)
            .next()
            .and_then(|el| parse_duration_str(&el.text().collect::<String>()));

        let tags: Vec<String> = card
            .select(&tag_sel)
            .map(|el| el.text().collect::<String>().trim().to_string())
            .filter(|t| !t.is_empty())
            .take(6)
            .collect();

        let source_url = card
            .select(&tag_sel)
            .next()
            .and_then(|el| el.value().attr("href"))
            .map(|href| {
                if href.starts_with("http") {
                    href.to_string()
                } else {
                    format!("https://mixkit.co{href}")
                }
            })
            .unwrap_or_else(|| "https://mixkit.co/free-sound-effects/".to_string());

        results.push(RawSfxCandidate {
            id: item_id,
            provider: "Mixkit".to_string(),
            title,
            author: "Mixkit".to_string(),
            license: LicenseInfo::mixkit_sfx_free(),
            approx_duration,
            preview_url: preview_url.to_string(),
            source_url,
            tags,
        });
    }

    (results, has_next)
}

/// Like [`parse_mixkit_page`], but tells "genuinely empty" apart from "markup changed".
///
/// * cards found and parsed -> `Ok`
/// * Mixkit's explicit "No results found" page -> `Ok(empty)`
/// * anything else that yields nothing -> [`AppError::ProviderLayoutChanged`]
pub fn parse_mixkit_checked(html: &str) -> Result<(Vec<RawSfxCandidate>, bool), AppError> {
    let (items, has_next) = parse_mixkit_page(html);
    if !items.is_empty() {
        return Ok((items, has_next));
    }

    let layout_err = |detail: &str| AppError::ProviderLayoutChanged {
        provider: "Mixkit".to_string(),
        detail: detail.to_string(),
    };

    if html.contains("No results found") {
        return Ok((Vec::new(), false));
    }

    let document = Html::parse_document(html);
    let card_sel = Selector::parse(".item-grid__item").expect("valid item selector");
    let container_sel = Selector::parse(".item-grid__items").expect("valid container selector");
    if document.select(&card_sel).next().is_some() {
        return Err(layout_err(
            "cards found but audio-player preview attributes are missing",
        ));
    }
    if document.select(&container_sel).next().is_some() {
        // Grid container present and truly empty: treat as an empty result.
        return Ok((Vec::new(), false));
    }
    Err(layout_err(
        "expected .item-grid__items / .item-grid__item not found and no 'No results found' marker",
    ))
}

fn score_token_relevance(candidate: &RawSfxCandidate, tokens: &[String]) -> usize {
    let title_l = candidate.title.to_ascii_lowercase();
    let tags_l: Vec<String> = candidate
        .tags
        .iter()
        .map(|t| t.to_ascii_lowercase())
        .collect();
    tokens
        .iter()
        .filter(|tok| {
            title_l.contains(tok.as_str()) || tags_l.iter().any(|t| t.contains(tok.as_str()))
        })
        .count()
}

#[async_trait]
impl SfxProvider for MixkitProvider {
    fn name(&self) -> &'static str {
        "Mixkit"
    }

    async fn search(
        &self,
        client: &reqwest::Client,
        params: &ProviderSearchParams,
    ) -> Result<ProviderPage, AppError> {
        let page = params.page.max(1);
        let tokens = tokenize_query(&params.query);

        // A layout error is only reported when no fallback URL produced results.
        let mut layout_err: Option<AppError> = None;

        let (mut candidates, has_next) = if tokens.is_empty() {
            let url = append_page_param("https://mixkit.co/free-sound-effects/", page);
            let html = self.fetch_html(client, &url).await?;
            parse_mixkit_checked(&html)?
        } else {
            let slug = tokens.join("-");
            let discover_base = format!(
                "https://mixkit.co/free-sound-effects/discover/{}/",
                urlencoding::encode(&slug)
            );
            let discover_url = append_page_param(&discover_base, page);
            let (mut found, mut next) = self
                .fetch_and_parse(client, &discover_url, &mut layout_err)
                .await?;

            if found.is_empty() {
                let category_base = format!(
                    "https://mixkit.co/free-sound-effects/{}/",
                    urlencoding::encode(&slug)
                );
                let category_url = append_page_param(&category_base, page);
                (found, next) = self
                    .fetch_and_parse(client, &category_url, &mut layout_err)
                    .await?;
            }

            if found.is_empty() && tokens.len() > 1 {
                let mut by_specificity = tokens.clone();
                by_specificity.sort_by(|a, b| b.len().cmp(&a.len()));
                for tok in by_specificity {
                    let base = format!(
                        "https://mixkit.co/free-sound-effects/discover/{}/",
                        urlencoding::encode(&tok)
                    );
                    let url = append_page_param(&base, page);
                    let (parsed, parsed_next) =
                        self.fetch_and_parse(client, &url, &mut layout_err).await?;
                    if !parsed.is_empty() {
                        found = parsed;
                        next = parsed_next;
                        break;
                    }
                }
            }

            if found.is_empty() {
                if let Some(err) = layout_err.take() {
                    return Err(err);
                }
            }

            if !tokens.is_empty() && !found.is_empty() {
                found.sort_by(|a, b| {
                    let sb = score_token_relevance(b, &tokens);
                    let sa = score_token_relevance(a, &tokens);
                    sb.cmp(&sa)
                });
            }

            (found, next)
        };

        if candidates.is_empty() {
            return Ok(ProviderPage {
                candidates: Vec::new(),
                has_next: false,
            });
        }

        let low = (params.min_duration - 1.0).max(0.0);
        let high = params.max_duration + 1.5;

        let filtered: Vec<RawSfxCandidate> = candidates
            .iter()
            .filter(|c| match c.approx_duration {
                Some(d) => d >= low && d <= high,
                None => true,
            })
            .cloned()
            .collect();

        if !filtered.is_empty() {
            candidates = filtered;
        } else if page <= 1 {
            candidates.sort_by(|a, b| {
                let da = a.approx_duration.unwrap_or(params.max_duration);
                let db = b.approx_duration.unwrap_or(params.max_duration);
                da.total_cmp(&db)
            });
        } else {
            candidates.clear();
        }

        candidates.truncate(params.page_size.max(1));
        Ok(ProviderPage {
            candidates,
            has_next: has_next && page < 20,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_mixkit_durations() {
        assert_eq!(parse_duration_str("0:02"), Some(2.0));
        assert_eq!(parse_duration_str(" 1:15 "), Some(75.0));
        assert_eq!(parse_duration_str("0:00"), Some(0.0));
        assert_eq!(parse_duration_str(""), None);
    }

    #[test]
    fn parses_mixkit_card_html_and_next_link() {
        let sample = r#"
        <html>
          <head><link rel="next" href="https://mixkit.co/free-sound-effects/discover/impact/?page=2" /></head>
          <body>
            <div class="item-grid__items">
              <div class="item-grid__item">
                <div class="item-grid-card">
                  <div
                    data-test-id="audio-player"
                    data-audio-player-preview-url-value="https://assets.mixkit.co/active_storage/sfx/1489/1489-preview.mp3"
                    data-audio-player-item-id-value="1489"
                  ></div>
                  <div class="item-grid-card__meta">
                    <h2 class="item-grid-card__title">  Air woosh  </h2>
                    <div class="meta-links__links">
                      <a class="meta-links__link" href="/free-sound-effects/woosh/">Woosh</a>
                      <a class="meta-links__link" href="/free-sound-effects/whoosh/">Whoosh</a>
                    </div>
                    <div class="item-grid-sfx-preview__meta-time" data-test-id="duration">0:02</div>
                  </div>
                </div>
              </div>
            </div>
          </body>
        </html>
        "#;

        let (parsed, has_next) = parse_mixkit_page(sample);
        assert!(has_next);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].id, "mk_1489");
        assert_eq!(parsed[0].title, "Air woosh");
        assert_eq!(
            parsed[0].preview_url,
            "https://assets.mixkit.co/active_storage/sfx/1489/1489-preview.mp3"
        );
        assert_eq!(parsed[0].approx_duration, Some(2.0));
        assert_eq!(parsed[0].tags, vec!["Woosh", "Whoosh"]);
    }

    const FIXTURE_OK: &str = include_str!("../../tests/fixtures/mixkit_whoosh.html");
    const FIXTURE_EMPTY: &str = include_str!("../../tests/fixtures/mixkit_no_results.html");

    #[test]
    fn fixture_real_page_parses_cards_with_license() {
        let (items, _) = parse_mixkit_checked(FIXTURE_OK).expect("fixture should parse");
        assert!(items.len() >= 10, "got {}", items.len());
        for it in &items {
            assert!(it.id.starts_with("mk_"));
            assert!(it.preview_url.starts_with("https://"));
            assert!(!it.title.is_empty());
            assert_eq!(it.license.id, "mixkit-sfx-free");
            assert!(it.approx_duration.is_some(), "duration missing for {}", it.title);
        }
    }

    #[test]
    fn fixture_no_results_page_is_empty_not_error() {
        let (items, has_next) = parse_mixkit_checked(FIXTURE_EMPTY).expect("empty is not an error");
        assert!(items.is_empty());
        assert!(!has_next);
    }

    #[test]
    fn renamed_selectors_raise_layout_changed() {
        let broken = FIXTURE_OK
            .replace("item-grid__item", "grid-entry")
            .replace("item-grid__items", "grid-entries");
        let err = parse_mixkit_checked(&broken).unwrap_err();
        assert!(matches!(err, AppError::ProviderLayoutChanged { .. }), "{err:?}");
        assert!(err.to_string().starts_with("PROVIDER_LAYOUT_CHANGED|Mixkit|"));
    }

    #[test]
    fn renamed_player_attributes_raise_layout_changed() {
        let broken = FIXTURE_OK.replace(
            "data-audio-player-preview-url-value",
            "data-audio-preview",
        );
        let err = parse_mixkit_checked(&broken).unwrap_err();
        assert!(matches!(err, AppError::ProviderLayoutChanged { .. }), "{err:?}");
    }
}
