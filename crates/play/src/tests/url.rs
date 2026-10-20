use std::cell::RefCell;

use crate::{Browser, Opening, URL_LIMIT, Url, UrlError};

#[test]
fn only_http_and_https_links_are_accepted() {
    for good in [
        "http://example.com",
        "https://example.com/path?query=1#part",
        "https://[::1]:8080/",
        "https://example.com/caf%C3%A9",
    ] {
        assert_eq!(Url::parse(good).map(|url| url.to_string()), Ok(good.into()));
    }
    assert_eq!(
        Url::parse("HTTPS://Example.COM:8443/A?Q=B").map(|url| url.to_string()),
        Ok("https://example.com:8443/A?Q=B".into())
    );
}

#[test]
fn every_other_scheme_is_refused() {
    for bad in [
        "file:///etc/passwd",
        "file://host/share",
        "javascript:alert(1)",
        "javascript://%0Aalert(1)",
        "JavaScript://example.com",
        "data:text/html,<script>alert(1)</script>",
        "mailto:someone@example.com",
        "steam://run/1",
        "ftp://example.com",
        "vbscript:msgbox",
        "ms-settings:",
        "example.com",
        "//example.com",
        "/etc/passwd",
        "C:\\Windows\\System32\\calc.exe",
        "-a https://example.com",
        " https://example.com",
        "httpx://example.com",
        "http:/example.com",
        "",
    ] {
        assert_eq!(Url::parse(bad), Err(UrlError::Scheme), "{bad:?}");
    }
}

#[test]
fn a_link_that_could_fool_the_launcher_or_the_player_is_refused() {
    for bad in [
        "https://example.com/a b",
        "https://example.com/\n--flag",
        "https://example.com/\0",
        "https://example.com/\u{7f}",
        "https://example.com/\u{202e}txt.exe",
        "https://ex\u{430}mple.com/",
        "https://example.com/\u{200b}",
        "https://example.com\\@evil.example",
        "https://user@evil.example",
        "https://good.example:pass@evil.example/",
        "https://",
        "https:///path",
        "https://:443/",
        "https://?q",
        "https://#frag",
        "https://example.com/\"--new-window",
        "https://example.com/<script>",
        "https://example.com/a>b",
        "https://example.com/`id`",
        "https://example.com/{a}",
        "https://example.com/a|b",
        "https://example.com/a^b",
    ] {
        assert!(
            matches!(Url::parse(bad), Err(UrlError::Malformed(_))),
            "{bad:?}"
        );
    }
    let long = format!("https://example.com/{}", "a".repeat(URL_LIMIT));
    assert!(matches!(Url::parse(&long), Err(UrlError::Malformed(_))));
}

struct Recorded(RefCell<Vec<String>>);

impl Browser for Recorded {
    fn open(&self, url: &Url) -> Opening {
        self.0.borrow_mut().push(url.as_str().into());
        Opening::done(Ok(()))
    }
}

#[test]
fn a_refused_link_never_reaches_the_browser() {
    let browser = Recorded(RefCell::new(Vec::new()));
    let open = |text: &str| Url::parse(text).map(|url| browser.open(&url));
    assert!(open("file:///etc/passwd").is_err());
    assert!(open("javascript:alert(1)").is_err());
    let mut opening = open("https://example.com").unwrap();
    assert_eq!(opening.poll(), Some(&Ok(())));
    assert_eq!(*browser.0.borrow(), ["https://example.com"]);
}

#[test]
fn an_opening_answers_once_its_browser_does() {
    let (answer, mut opening) = Opening::pending();
    assert_eq!(opening.poll(), None);
    answer
        .send(Err(UrlError::Failed("no launcher".into())))
        .unwrap();
    drop(answer);
    assert_eq!(
        opening.poll(),
        Some(&Err(UrlError::Failed("no launcher".into())))
    );
    assert_eq!(
        opening.poll(),
        Some(&Err(UrlError::Failed("no launcher".into())))
    );
    let (answer, mut gone) = Opening::pending();
    drop(answer);
    assert!(matches!(gone.poll(), Some(Err(UrlError::Failed(_)))));
}
