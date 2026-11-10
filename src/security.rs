use url::Url;

pub fn origin_allowed(origin: Option<&str>, authority: &str, scheme: &str) -> bool {
    origin.is_none_or(|origin| {
        Url::parse(origin).is_ok_and(|url| {
            url.scheme() == scheme
                && url.username().is_empty()
                && url.password().is_none()
                && url.path() == "/"
                && url.query().is_none()
                && url.fragment().is_none()
                && url[url::Position::BeforeHost..url::Position::AfterPort] == *authority
        })
    })
}

pub fn modern_version_allowed(version: Option<&str>) -> bool {
    version.is_none_or(|v| matches!(v, "2025-03-26" | "2025-06-18"))
}
