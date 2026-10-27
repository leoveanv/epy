use crate::{Error, Result};
use std::path::{Path, PathBuf};
use url::Url;

#[derive(Clone, Debug)]
pub enum BookLocation {
    Local(PathBuf),
    Web(Url),
}

pub(crate) fn validate_web_url(url: &Url) -> Result<()> {
    if !matches!(url.scheme(), "http" | "https")
        || !url.has_host()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(Error::Compatibility(
            "Use an HTTP/HTTPS URL without embedded credentials.".into(),
        ));
    }
    Ok(())
}

impl BookLocation {
    pub fn local(path: impl Into<PathBuf>) -> Self {
        Self::Local(path.into())
    }
    pub fn parse(value: &str) -> Result<Self> {
        if Path::new(value).is_file() {
            return Ok(Self::local(value));
        }
        if value.contains("://")
            || value.to_ascii_lowercase().starts_with("http:")
            || value.to_ascii_lowercase().starts_with("https:")
        {
            let url = Url::parse(value)
                .map_err(|error| Error::Compatibility(format!("Invalid ebook URL: {error}")))?;
            validate_web_url(&url)?;
            Ok(Self::Web(url))
        } else {
            Ok(Self::local(value))
        }
    }
    pub fn storage_key(&self) -> Result<String> {
        match self {
            Self::Web(url) => Ok(url.to_string()),
            Self::Local(path) => path.to_str().map(str::to_owned).ok_or_else(|| {
                Error::Compatibility("The ebook path must be valid Unicode.".into())
            }),
        }
    }
}

impl From<PathBuf> for BookLocation {
    fn from(path: PathBuf) -> Self {
        Self::local(path)
    }
}
