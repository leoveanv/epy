use super::BookMetadata;
use super::BookNavigation;
use super::dispatch::{BookAsset, BookBackend, BookDescriptor, BookOpenOptions, ResourceTarget};
use super::location::validate_web_url;
use crate::cancellation::Cancellation;
use crate::document::html::{HtmlToken, HtmlTokenizer, unescape_html};
use crate::{Error, Result};
use reqwest::{Client, Url};
use std::sync::OnceLock;
use std::time::Duration;
use tokio::runtime::{Builder, Runtime};

pub(super) struct WebBook {
    descriptor: BookDescriptor,
    html: String,
    base: Url,
    client: Client,
    cancellation: Cancellation,
    resource_bytes_max: u64,
}
struct WebResponse {
    url: Url,
    content_type: String,
    bytes: Vec<u8>,
}

fn runtime() -> Result<&'static Runtime> {
    static RUNTIME: OnceLock<std::result::Result<Runtime, std::io::Error>> = OnceLock::new();
    RUNTIME
        .get_or_init(|| {
            Builder::new_multi_thread()
                .worker_threads(2)
                .max_blocking_threads(4)
                .enable_all()
                .build()
        })
        .as_ref()
        .map_err(|error| Error::Compatibility(format!("Cannot start the network runtime: {error}")))
}

fn network_error(error: reqwest::Error) -> Error {
    Error::Compatibility(format!("Network request failed: {error}"))
}

async fn canceled(cancellation: &Cancellation) {
    while !cancellation.is_canceled() {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn request(client: &Client, url: &Url, limit: u64) -> Result<WebResponse> {
    validate_web_url(url)?;
    let mut response = client
        .get(url.clone())
        .send()
        .await
        .map_err(network_error)?
        .error_for_status()
        .map_err(network_error)?;
    if response
        .content_length()
        .is_some_and(|length| length > limit)
    {
        return Err(Error::Compatibility(
            "The web response exceeds its size limit.".into(),
        ));
    }
    let url = response.url().clone();
    validate_web_url(&url)?;
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(network_error)? {
        if (bytes.len() as u64).saturating_add(chunk.len() as u64) > limit {
            return Err(Error::Compatibility(
                "The web response exceeds its decoded size limit.".into(),
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(WebResponse {
        url,
        content_type,
        bytes,
    })
}

fn fetch(
    client: &Client,
    url: &Url,
    limit: u64,
    cancellation: &Cancellation,
) -> Result<WebResponse> {
    cancellation.check()?;
    runtime()?.block_on(async { tokio::select! { biased; _ = canceled(cancellation) => Err(Error::Canceled), result = request(client, url, limit) => result } })
}

fn decode_html(response: &WebResponse) -> Result<String> {
    let label = response
        .content_type
        .split(';')
        .map(str::trim)
        .find_map(|part| part.strip_prefix("charset="))
        .map(|label| label.trim_matches(['\'', '"']));
    let (encoding, skip) = encoding_rs::Encoding::for_bom(&response.bytes).unwrap_or((
        label
            .and_then(|label| encoding_rs::Encoding::for_label(label.as_bytes()))
            .unwrap_or(encoding_rs::UTF_8),
        0,
    ));
    let (text, errors) = encoding.decode_without_bom_handling(&response.bytes[skip..]);
    if errors {
        return Err(Error::Compatibility(
            "The web document contains invalid text for its character encoding.".into(),
        ));
    }
    Ok(text.into_owned())
}

fn title(html: &str) -> Option<String> {
    let mut active = false;
    let mut title = String::new();
    for token in HtmlTokenizer::new(html) {
        match token {
            HtmlToken::Start(tag) if tag.name == "title" => active = true,
            HtmlToken::End(tag) if tag == "title" => break,
            HtmlToken::Data(data) if active => title.push_str(&unescape_html(&data)),
            _ => {}
        }
    }
    let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
    (!title.is_empty()).then_some(title)
}

impl WebBook {
    pub fn open(url: &Url, options: &BookOpenOptions) -> Result<Self> {
        validate_web_url(url)?;
        let _guard = runtime()?.enter();
        let client = Client::builder()
            .user_agent(concat!("ers/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                if attempt.previous().len() >= 10 || validate_web_url(attempt.url()).is_err() {
                    attempt.error("Unsafe URL or too many redirects")
                } else {
                    attempt.follow()
                }
            }))
            .build()
            .map_err(network_error)?;
        drop(_guard);
        let response = fetch(
            &client,
            url,
            options.limits.resource_bytes_max,
            &options.cancellation,
        )?;
        let html = decode_html(&response)?;
        let descriptor = BookDescriptor {
            metadata: BookMetadata {
                title: title(&html),
                source: Some(url.to_string()),
                format: Some("HTML".into()),
                ..BookMetadata::default()
            },
            contents: vec!["_".into()],
            spine_len: 1,
            toc: Vec::new(),
            navigation: BookNavigation::default(),
        };
        Ok(Self {
            descriptor,
            html,
            base: response.url,
            client,
            cancellation: options.cancellation.clone(),
            resource_bytes_max: options.limits.resource_bytes_max,
        })
    }
}

impl BookBackend for WebBook {
    fn descriptor(&self) -> &BookDescriptor {
        &self.descriptor
    }
    fn raw_text(&mut self, id: &str) -> Result<String> {
        if id != "_" {
            return Err(Error::Compatibility(
                "The web document does not exist.".into(),
            ));
        }
        Ok(self.html.clone())
    }
    fn asset(&mut self, id: &str) -> Result<BookAsset> {
        let url = self
            .base
            .join(id)
            .map_err(|error| Error::Compatibility(format!("Invalid web resource URL: {error}")))?;
        let response = fetch(
            &self.client,
            &url,
            self.resource_bytes_max,
            &self.cancellation,
        )?;
        let name = response
            .url
            .path_segments()
            .and_then(Iterator::last)
            .filter(|name| !name.is_empty())
            .unwrap_or("image")
            .to_owned();
        Ok(BookAsset {
            name,
            bytes: response.bytes,
        })
    }
    fn resolve_resource(&self, _: &str, href: &str) -> Result<ResourceTarget> {
        let mut url = self
            .base
            .join(href)
            .map_err(|error| Error::Compatibility(format!("Invalid web resource URL: {error}")))?;
        validate_web_url(&url)?;
        let fragment = url.fragment().map(|value| {
            percent_encoding::percent_decode_str(value)
                .decode_utf8_lossy()
                .into_owned()
        });
        url.set_fragment(None);
        let mut base = self.base.clone();
        base.set_fragment(None);
        Ok(ResourceTarget {
            content_id: if url == base {
                "_".into()
            } else {
                url.to_string()
            },
            fragment,
        })
    }
}
