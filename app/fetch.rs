use miette::*;
use reqwest::Client;
use std::future::Future;

#[cfg(not(feature = "local"))]
pub async fn string<'h, H>(client: &Client, url: &str, hdrs: H) -> Result<String>
where
    H: IntoIterator<Item = (&'h str, String)>,
{
    get(client, url, hdrs, |x| async { x.text().await }).await
}

#[cfg(not(feature = "local"))]
pub async fn json<'h, T, H>(client: &Client, url: &str, hdrs: H) -> Result<T>
where
    T: for<'a> serde::Deserialize<'a>,
    H: IntoIterator<Item = (&'h str, String)>,
{
    get(client, url, hdrs, |x| async { x.json().await }).await
}

async fn get<F, H, K, V, T, O>(client: &Client, url: &str, hdrs: H, f: F) -> Result<T>
where
    H: IntoIterator<Item = (K, V)>,
    K: TryInto<reqwest::header::HeaderName>,
    K::Error: std::error::Error + Send + Sync + 'static,
    V: TryInto<reqwest::header::HeaderValue>,
    V::Error: std::error::Error + Send + Sync + 'static,
    F: FnOnce(reqwest::Response) -> O,
    O: Future<Output = reqwest::Result<T>>,
{
    let mut headers = reqwest::header::HeaderMap::new();
    for (k, v) in hdrs {
        headers.insert(
            k.try_into().into_diagnostic()?,
            v.try_into().into_diagnostic()?,
        );
    }

    let resp = client
        .get(url)
        .headers(headers)
        .send()
        .await
        .into_diagnostic()
        .wrap_err_with(|| format!("URL: {url}"))
        .wrap_err("failed to send GET")?;
    resp.error_for_status_ref()
        .into_diagnostic()
        .wrap_err_with(|| format!("URL: {url}"))
        .wrap_err_with(|| format!("error response code {}", resp.status()))?;
    f(resp)
        .await
        .into_diagnostic()
        .wrap_err_with(|| format!("URL: {url}"))
        .wrap_err("failed to ready body")
}

// ##### LOCAL FILES ############################################
#[cfg(feature = "local")]
pub async fn string<'h, H>(client: &Client, url: &str, hdrs: H) -> Result<String>
where
    H: IntoIterator<Item = (&'h str, String)>,
{
    let url_short = url.split('?').next().unwrap();
    let path = local::FILES
        .iter()
        .find_map(|(u, p)| url_short.eq(*u).then_some(p))
        .ok_or_else(|| miette!("no local file defined for {}", url))?;
    tokio::fs::read_to_string(path)
        .await
        .into_diagnostic()
        .wrap_err_with(|| format!("failed to read local file at {path}"))
}

#[cfg(feature = "local")]
pub async fn json<'h, T, H>(client: &Client, url: &str, hdrs: H) -> Result<T>
where
    T: for<'a> serde::Deserialize<'a>,
    H: IntoIterator<Item = (&'h str, String)>,
{
    let s = string(client, url, hdrs).await?;
    serde_json::from_str(&s)
        .into_diagnostic()
        .wrap_err("JSON failure")
}

#[cfg(feature = "local")]
mod local {
    pub const FILES: &[(&str, &str)] = &[
        (
            "https://calendar.google.com/calendar/ical/path-to-cal",
            "./kurt-cal.ics",
        ),
        ("https://api.open-meteo.com/v1/forecast", "./weather.json"),
        (
            "https://api.stormglass.io/v2/astronomy/point",
            "./moon.json",
        ),
    ];
}
