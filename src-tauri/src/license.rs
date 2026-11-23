//! License metadata attached to every sound.
//!
//! Rule of thumb: only report what the source actually told us. Anything we can't
//! map with certainty becomes `unknown` with no attribution/commercial claims.

use serde::{Deserialize, Serialize};

pub const LICENSE_UNKNOWN: &str = "unknown";
pub const LICENSE_LOCAL: &str = "local";

pub const MIXKIT_LICENSE_URL: &str = "https://mixkit.co/license/#sfxFree";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LicenseInfo {
    /// Stable machine id: `cc0`, `cc-by`, `cc-by-nc`, `mixkit-sfx-free`, `unknown`, `local`...
    pub id: String,
    /// Human readable name.
    pub name: String,
    pub url: String,
    pub requires_attribution: bool,
    /// `None` = not known / not stated.
    pub commercial_use_allowed: Option<bool>,
}

impl LicenseInfo {
    pub fn unknown() -> Self {
        Self {
            id: LICENSE_UNKNOWN.to_string(),
            name: "Unknown".to_string(),
            url: String::new(),
            requires_attribution: false,
            commercial_use_allowed: None,
        }
    }

    /// Sounds from the user's own disk: the app makes no licensing claims.
    pub fn local_file() -> Self {
        Self {
            id: LICENSE_LOCAL.to_string(),
            name: "Local file".to_string(),
            url: String::new(),
            requires_attribution: false,
            commercial_use_allowed: None,
        }
    }

    /// Mixkit Sound Effects Free License (royalty-free, no attribution required).
    pub fn mixkit_sfx_free() -> Self {
        Self {
            id: "mixkit-sfx-free".to_string(),
            name: "Mixkit Sound Effects Free License".to_string(),
            url: MIXKIT_LICENSE_URL.to_string(),
            requires_attribution: false,
            commercial_use_allowed: Some(true),
        }
    }

    /// Maps the `license` field of the Freesound API (a Creative Commons URL).
    pub fn from_freesound_license(raw: &str) -> Self {
        let url = raw.trim();
        let lower = url.to_ascii_lowercase();
        if lower.is_empty() {
            return Self::unknown();
        }
        let version = cc_version(&lower);
        let with_version = |base: &str| match &version {
            Some(v) => format!("{base} {v}"),
            None => base.to_string(),
        };

        if lower.contains("publicdomain/zero") {
            return Self {
                id: "cc0".to_string(),
                name: "Creative Commons 0".to_string(),
                url: url.to_string(),
                requires_attribution: false,
                commercial_use_allowed: Some(true),
            };
        }
        if lower.contains("/licenses/by-nc/") {
            return Self {
                id: "cc-by-nc".to_string(),
                name: with_version("CC BY-NC"),
                url: url.to_string(),
                requires_attribution: true,
                commercial_use_allowed: Some(false),
            };
        }
        if lower.contains("/licenses/by/") {
            return Self {
                id: "cc-by".to_string(),
                name: with_version("CC BY"),
                url: url.to_string(),
                requires_attribution: true,
                commercial_use_allowed: Some(true),
            };
        }
        if lower.contains("/licenses/sampling+/") {
            // Legacy license: credit is required, commercial scope is not stated here.
            return Self {
                id: "cc-sampling-plus".to_string(),
                name: "CC Sampling+ 1.0".to_string(),
                url: url.to_string(),
                requires_attribution: true,
                commercial_use_allowed: None,
            };
        }

        Self {
            url: if lower.starts_with("http") {
                url.to_string()
            } else {
                String::new()
            },
            ..Self::unknown()
        }
    }

    /// Re-derives license info for rows cached before license columns existed.
    pub fn from_legacy(provider: &str, legacy_license: &str) -> Self {
        let trimmed = legacy_license.trim();
        if trimmed.starts_with("http") {
            return Self::from_freesound_license(trimmed);
        }
        if provider.eq_ignore_ascii_case("mixkit") && trimmed.to_ascii_lowercase().contains("mixkit")
        {
            return Self::mixkit_sfx_free();
        }
        if trimmed == LICENSE_LOCAL {
            return Self::local_file();
        }
        Self::unknown()
    }

    /// True only when we positively know the sound is usable commercially without credit.
    pub fn is_commercial_no_attribution(&self) -> bool {
        self.commercial_use_allowed == Some(true) && !self.requires_attribution
    }
}

fn cc_version(lower_url: &str) -> Option<String> {
    let trimmed = lower_url.trim_end_matches('/');
    let last = trimmed.rsplit('/').next()?;
    if last.chars().all(|c| c.is_ascii_digit() || c == '.') && last.contains('.') {
        Some(last.to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_freesound_cc_urls() {
        let cc0 = LicenseInfo::from_freesound_license(
            "http://creativecommons.org/publicdomain/zero/1.0/",
        );
        assert_eq!(cc0.id, "cc0");
        assert!(cc0.is_commercial_no_attribution());

        let by = LicenseInfo::from_freesound_license("https://creativecommons.org/licenses/by/4.0/");
        assert_eq!(by.id, "cc-by");
        assert_eq!(by.name, "CC BY 4.0");
        assert!(by.requires_attribution);
        assert_eq!(by.commercial_use_allowed, Some(true));
        assert!(!by.is_commercial_no_attribution());

        let nc = LicenseInfo::from_freesound_license(
            "http://creativecommons.org/licenses/by-nc/3.0/",
        );
        assert_eq!(nc.id, "cc-by-nc");
        assert_eq!(nc.commercial_use_allowed, Some(false));
    }

    #[test]
    fn unrecognized_license_is_unknown_without_claims() {
        for raw in ["", "Freesound CC", "something-else"] {
            let l = LicenseInfo::from_freesound_license(raw);
            assert_eq!(l.id, LICENSE_UNKNOWN);
            assert!(!l.requires_attribution);
            assert_eq!(l.commercial_use_allowed, None);
            assert!(!l.is_commercial_no_attribution());
        }
    }

    #[test]
    fn legacy_rows_are_remapped_conservatively() {
        assert_eq!(
            LicenseInfo::from_legacy("Mixkit", "Mixkit Free License").id,
            "mixkit-sfx-free"
        );
        assert_eq!(LicenseInfo::from_legacy("Freesound", "Freesound CC").id, LICENSE_UNKNOWN);
        assert_eq!(
            LicenseInfo::from_legacy("Freesound", "http://creativecommons.org/licenses/by-nc/4.0/")
                .id,
            "cc-by-nc"
        );
    }
}
