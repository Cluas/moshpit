//! Home's words that come with a host's data rather than the screen's own
//! copy: said in the language the app shows (`locale`), as the iOS app
//! said them with `String(localized:)`, for the one Home builder
//! (`moshpit_app::home`) both platforms run.

use crate::tl;

/// The herdr version skew a card's banner names (`HerdrVersionSkew.notice`):
/// the host's PATH holds an older herdr than its server, or the server is
/// the older one.
pub fn herdr_skew(client_is_older: bool, client: &str, server: &str, locale: &str) -> String {
    if client_is_older {
        tl!(
            "herdr on the host's PATH is %@, its server %@ — update that copy",
            locale,
            arg1 = client,
            arg2 = server
        )
    } else {
        tl!(
            "The host's herdr server is %@, older than its herdr %@",
            locale,
            arg1 = server,
            arg2 = client
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_skew_is_said_in_the_apps_language() {
        assert_eq!(
            herdr_skew(true, "0.9.4", "0.9.5", "en"),
            "herdr on the host's PATH is 0.9.4, its server 0.9.5 \u{2014} update that copy"
        );
        assert_eq!(
            herdr_skew(false, "0.9.5", "0.9.4", "en"),
            "The host's herdr server is 0.9.4, older than its herdr 0.9.5"
        );
        let zh = herdr_skew(true, "0.9.4", "0.9.5", "zh-Hans");
        assert!(zh.contains("0.9.4") && zh.contains("0.9.5"), "{zh}");
        assert!(
            !zh.starts_with("zh-Hans.") && !zh.contains("PATH is"),
            "{zh}"
        );
    }
}
