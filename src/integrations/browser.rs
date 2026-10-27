use crate::{Error, Result};
use url::Url;

#[derive(Clone, Debug)]
pub struct BrowserCommand {
    pub program: String,
    pub arguments: Vec<String>,
}

impl BrowserCommand {
    pub fn parse(template: &str, url: &Url) -> Result<Self> {
        validate_url(url)?;
        let mut words = shell_words::split(template)
            .map_err(|error| Error::Compatibility(format!("Invalid browser command: {error}")))?
            .into_iter();
        let program = words
            .next()
            .filter(|program| !program.is_empty())
            .ok_or_else(|| Error::Compatibility("The browser command is empty.".into()))?;
        let mut arguments = words
            .map(|word| word.replace("%s", url.as_str()))
            .collect::<Vec<_>>();
        if !template.contains("%s") {
            arguments.push(url.as_str().into());
        }
        Ok(Self { program, arguments })
    }

    fn execute(&self) -> std::io::Result<bool> {
        std::process::Command::new(&self.program)
            .args(&self.arguments)
            .status()
            .map(|status| status.success())
    }
}

fn validate_url(url: &Url) -> Result<()> {
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(Error::Compatibility(
            "Only HTTP and HTTPS links can open in a browser.".into(),
        ));
    }
    Ok(())
}

pub fn open_browser(url: &Url, preference: Option<&str>) -> Result<()> {
    validate_url(url)?;
    if let Some(preference) = preference {
        for template in preference
            .split(if cfg!(windows) { ';' } else { ':' })
            .filter(|template| !template.is_empty())
        {
            if let Ok(command) = BrowserCommand::parse(template, url)
                && command.execute().unwrap_or(false)
            {
                return Ok(());
            }
        }
    }
    webbrowser::open(url.as_str()).map_err(|error| {
        Error::Compatibility(format!(
            "The default browser could not open the link: {error}"
        ))
    })
}
