use crate::export::Credentials;
use serde::Serialize;
use tauri::webview::Cookie;

const NAMES: [&str; 4] = ["sessionid", "tt-target-idc", "msToken", "s_v_web_id"];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Present,
    Missing,
    Expired,
    Ambiguous,
    Invalid,
}

impl State {
    fn label(self) -> &'static str {
        match self {
            Self::Present => "present",
            Self::Missing => "missing",
            Self::Expired => "expired",
            Self::Ambiguous => "ambiguous",
            Self::Invalid => "invalid",
        }
    }
}

#[derive(Clone, Serialize)]
pub struct ReadinessItem {
    pub name: &'static str,
    pub state: State,
}

#[derive(Clone, Serialize)]
pub struct Readiness {
    pub items: [ReadinessItem; 4],
    pub complete: bool,
}

pub fn inspect(cookies: &[Cookie<'static>], now: i64) -> Readiness {
    let items = std::array::from_fn(|index| ReadinessItem {
        name: NAMES[index],
        state: select(cookies, index, now).0,
    });
    let complete = items.iter().all(|item| item.state == State::Present);
    Readiness { items, complete }
}

pub fn collect(cookies: &[Cookie<'static>], now: i64) -> Result<Credentials, String> {
    let selected = std::array::from_fn::<_, 4, _>(|index| select(cookies, index, now));
    for (index, (state, _)) in selected.iter().enumerate() {
        if *state != State::Present {
            return Err(format!("{}: {}", NAMES[index], state.label()));
        }
    }
    let value = |index: usize| selected[index].1.unwrap_or_default().to_owned();
    Ok(Credentials {
        session_id: value(0),
        target_idc: value(1),
        ms_token: value(2),
        verify_fp: value(3),
    })
}

fn in_scope(cookie: &Cookie<'_>) -> bool {
    cookie.path() == Some("/")
        && cookie.domain().is_some_and(|domain| {
            domain.eq_ignore_ascii_case("www.tiktok.com")
                || domain.eq_ignore_ascii_case("tiktok.com")
        })
}

fn select<'a>(cookies: &'a [Cookie<'static>], index: usize, now: i64) -> (State, Option<&'a str>) {
    let active = |cookie: &&Cookie<'static>| {
        cookie.name() == NAMES[index]
            && in_scope(cookie)
            && cookie
                .expires_datetime()
                .is_none_or(|expires| expires.unix_timestamp() > now)
    };
    // The bot reuses msToken for www and webcast; prefer their shared parent scope.
    let shared_token = index == 2
        && cookies.iter().filter(active).any(|cookie| {
            cookie
                .domain()
                .is_some_and(|domain| domain.eq_ignore_ascii_case("tiktok.com"))
        });
    let mut value = None;
    for cookie in cookies.iter().filter(active).filter(|cookie| {
        !shared_token
            || cookie
                .domain()
                .is_some_and(|domain| domain.eq_ignore_ascii_case("tiktok.com"))
    }) {
        if value.is_some_and(|previous| previous != cookie.value()) {
            return (State::Ambiguous, None);
        }
        value = Some(cookie.value());
    }
    match value {
        None if cookies
            .iter()
            .any(|cookie| cookie.name() == NAMES[index] && in_scope(cookie)) =>
        {
            (State::Expired, None)
        }
        None => (State::Missing, None),
        Some(value) if !valid_value(index, value) => (State::Invalid, None),
        Some(value) => (State::Present, Some(value)),
    }
}

fn valid_value(index: usize, value: &str) -> bool {
    let limit = [256, 64, 1024, 200][index];
    !value.is_empty()
        && value.len() <= limit
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || byte == b'_'
                || byte == b'-'
                || match index {
                    0 | 3 => byte == b'.',
                    2 => matches!(byte, b'=' | b'+' | b'/'),
                    _ => false,
                }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000;

    fn cookie(name: &'static str, value: impl Into<String>) -> Cookie<'static> {
        Cookie::build((name, value.into()))
            .domain(".tiktok.com")
            .path("/")
            .secure(true)
            .http_only(true)
            .build()
    }

    fn complete() -> Vec<Cookie<'static>> {
        vec![
            cookie("sessionid", "Session_1.a-b"),
            cookie("tt-target-idc", "eu_1-a"),
            cookie("msToken", "Token_1=+/-"),
            cookie("s_v_web_id", "verify_1.a-b"),
        ]
    }

    fn state(cookies: &[Cookie<'static>], index: usize) -> State {
        inspect(cookies, NOW).items[index].state
    }

    fn expired(mut cookie: Cookie<'static>) -> Cookie<'static> {
        let expiry = Cookie::parse("temporary=value; Expires=Fri, 15 Jan 2027 08:00:00 GMT")
            .unwrap()
            .expires()
            .unwrap();
        cookie.set_expires(expiry);
        cookie
    }

    #[test]
    fn complete_snapshot_maps_fields_and_serializes_only_readiness() {
        let cookies = complete();
        let readiness = inspect(&cookies, NOW);
        assert!(readiness.complete);
        assert_eq!(readiness.items.map(|item| item.name), NAMES);
        let json = serde_json::to_value(inspect(&cookies, NOW)).unwrap();
        assert_eq!(json["items"][0]["state"], "present");
        assert_eq!(json["items"][0].as_object().unwrap().len(), 2);
        assert!(!json.to_string().contains("Session_1"));
        let credentials = collect(&cookies, NOW).unwrap();
        assert_eq!(credentials.session_id, "Session_1.a-b");
        assert_eq!(credentials.target_idc, "eu_1-a");
        assert_eq!(credentials.ms_token, "Token_1=+/-");
        assert_eq!(credentials.verify_fp, "verify_1.a-b");
    }

    #[test]
    fn missing_or_partial_snapshot_cannot_export() {
        assert_eq!(state(&[], 0), State::Missing);
        for missing in 0..NAMES.len() {
            let mut cookies = complete();
            cookies.remove(missing);
            assert!(!inspect(&cookies, NOW).complete);
            assert_eq!(state(&cookies, missing), State::Missing);
            assert!(collect(&cookies, NOW).is_err());
        }
    }

    #[test]
    fn expiry_boundary_and_session_cookies() {
        let cookies = vec![expired(cookie("sessionid", "value"))];
        assert_eq!(cookies[0].expires_datetime().unwrap().unix_timestamp(), NOW);
        for (delta, expected) in [
            (-1, State::Present),
            (0, State::Expired),
            (1, State::Expired),
        ] {
            assert_eq!(inspect(&cookies, NOW + delta).items[0].state, expected);
        }
        assert_eq!(state(&[cookie("sessionid", "value")], 0), State::Present);
    }

    #[test]
    fn expired_values_never_conflict_with_live_values_or_each_other() {
        for (index, name) in NAMES.iter().copied().enumerate() {
            let mut cookies = vec![
                expired(cookie(name, "Old_A")),
                expired(cookie(name, "Old_B")),
            ];
            assert_eq!(state(&cookies, index), State::Expired);
            cookies.push(cookie(name, "Current"));
            for _ in 0..2 {
                assert_eq!(state(&cookies, index), State::Present);
                assert_eq!(select(&cookies, index, NOW).1, Some("Current"));
                cookies.reverse();
            }
        }
    }

    #[test]
    fn equal_duplicates_are_allowed_but_conflicts_fail_closed() {
        let mut cookies = complete();
        cookies.push(cookie("sessionid", "Session_1.a-b"));
        assert!(collect(&cookies, NOW).is_ok());
        cookies.push(expired(cookie("sessionid", "Session_1.a-b")));
        assert!(collect(&cookies, NOW).is_ok());
        cookies.push(expired(cookie("sessionid", "other-value")));
        assert!(collect(&cookies, NOW).is_ok());
        cookies.push(cookie("sessionid", "other-value"));
        assert_eq!(state(&cookies, 0), State::Ambiguous);
        let error = collect(&cookies, NOW).err().unwrap();
        assert_eq!(error, "sessionid: ambiguous");
        assert!(!error.contains("other-value"));
    }

    #[test]
    fn browser_token_prefers_live_shared_scope_independent_of_order() {
        let mut cookies = complete();
        let mut host_token = cookie("msToken", "Host_token");
        host_token.set_domain("www.tiktok.com");
        cookies.push(host_token.clone());
        for _ in 0..2 {
            assert!(inspect(&cookies, NOW).complete);
            assert_eq!(collect(&cookies, NOW).unwrap().ms_token, "Token_1=+/-");
            cookies.reverse();
        }
        cookies.push(cookie("msToken", "Token_1=+/-"));
        assert_eq!(collect(&cookies, NOW).unwrap().ms_token, "Token_1=+/-");
        cookies.retain(|cookie| cookie.name() != "msToken");
        cookies.push(expired(cookie("msToken", "Expired_parent")));
        cookies.push(host_token);
        assert_eq!(collect(&cookies, NOW).unwrap().ms_token, "Host_token");
        cookies.push(cookie("msToken", "invalid;parent"));
        assert_eq!(state(&cookies, 2), State::Invalid);
        assert!(collect(&cookies, NOW).is_err());
    }

    #[test]
    fn live_conflicts_in_selected_token_scope_and_account_cookies_still_block_export() {
        for domain in [".tiktok.com", "www.tiktok.com"] {
            let mut cookies = complete();
            cookies.retain(|cookie| cookie.name() != "msToken");
            for value in ["Token_A", "Token_B"] {
                let mut token = cookie("msToken", value);
                token.set_domain(domain);
                cookies.push(token);
            }
            if domain == ".tiktok.com" {
                let mut fallback = cookie("msToken", "Valid_host_fallback");
                fallback.set_domain("www.tiktok.com");
                cookies.push(fallback);
            }
            assert_eq!(state(&cookies, 2), State::Ambiguous);
            assert!(collect(&cookies, NOW).is_err());
        }
        for index in [0, 1, 3] {
            let mut cookies = complete();
            let mut conflict = cookie(NAMES[index], "Other_value");
            conflict.set_domain("www.tiktok.com");
            cookies.push(conflict);
            assert_eq!(state(&cookies, index), State::Ambiguous);
            assert!(collect(&cookies, NOW).is_err());
        }
    }

    #[test]
    fn validators_match_ascii_bot_contract_and_lengths() {
        for (index, maximum) in [256, 64, 1024, 200].into_iter().enumerate() {
            assert!(valid_value(index, &"a".repeat(maximum)));
            for invalid in [
                "".to_owned(),
                "a".repeat(maximum + 1),
                "space value".into(),
                "line\nvalue".into(),
                "é".into(),
                "semicolon;".into(),
                "colon:".into(),
            ] {
                assert!(!valid_value(index, &invalid));
                assert_eq!(
                    state(&[cookie(NAMES[index], invalid)], index),
                    State::Invalid
                );
            }
        }
        assert!(!valid_value(1, "has.dot"));
        assert!(!valid_value(2, "has.dot"));
        assert!(!valid_value(0, "has+plus"));
        assert!(!valid_value(3, "has/slash"));
    }

    #[test]
    fn unrelated_names_domains_and_paths_do_not_contribute_or_conflict() {
        let mut cookies = complete();
        for domain in ["evil.example", "tiktok.com.evil.example", "api.tiktok.com"] {
            let mut ignored = cookie("sessionid", "conflicting");
            ignored.set_domain(domain);
            cookies.push(ignored);
        }
        for path in ["/login", "/account", ""] {
            let mut ignored = cookie("sessionid", "conflicting");
            ignored.set_path(path);
            cookies.push(ignored);
        }
        let mut no_domain = cookie("sessionid", "conflicting");
        no_domain.unset_domain();
        cookies.push(no_domain);
        let mut no_path = cookie("sessionid", "conflicting");
        no_path.unset_path();
        cookies.push(no_path);
        cookies.push(cookie("other", "arbitrary; value"));
        assert!(collect(&cookies, NOW).is_ok());
        let mut exact_host = cookie("sessionid", "Session_1.a-b");
        exact_host.set_domain("www.tiktok.com");
        cookies.push(exact_host);
        assert!(collect(&cookies, NOW).is_ok());
    }

    #[test]
    fn snapshots_do_not_accumulate_credentials() {
        let first = complete();
        assert!(inspect(&first, NOW).complete);
        let second = vec![cookie("sessionid", "new-session")];
        assert!(!inspect(&second, NOW).complete);
        assert!(collect(&second, NOW).is_err());
        assert!(!inspect(&first[..2], NOW).complete);
        assert!(!inspect(&first[2..], NOW).complete);
    }
}
