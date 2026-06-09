use std::{
    fmt,
    fs::read_to_string,
    io::{self, Write as _},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, RwLock},
};

use ghrepo::GHRepo;
use reqwest::header::CONTENT_TYPE;
use url::Url;

use crate::{
    OK_END, VERSION,
    document::Document,
    error::{Error, NavigationError},
};

#[derive(Clone, Debug, PartialEq)]
pub enum DocumentSource {
    File {
        path: PathBuf,
        basepath: Option<PathBuf>,
    },
    Stdin {
        text: Option<String>,
    },
    Github {
        repo: GHRepo,
        branch: String,
    },
    HyperText {
        url: Url,
    },
    BuiltIn(BuiltIn),
    Image {
        path: PathBuf,
    },
    Pdf {
        path: PathBuf,
    },
}

impl DocumentSource {
    pub fn return_text(self, returned_text: String) -> Option<Self> {
        match self {
            DocumentSource::Stdin { .. } => Some(DocumentSource::Stdin {
                text: Some(returned_text),
            }),
            // TODO: Github and HyperText should also return the text to avoid re-fetch.
            // File is OK to just reload from disk.
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BuiltIn {
    Help,
    HelpConfiguration,
    Welcome,
    Changelog,
}
impl BuiltIn {
    pub fn source(&self) -> (DocumentSource, Option<String>) {
        match self {
            BuiltIn::Help => {
                const HELP_MD: &str = include_str!("../assets/docs/help.md");
                (DocumentSource::BuiltIn(*self), Some(String::from(HELP_MD)))
            }
            BuiltIn::HelpConfiguration => {
                const HELP_CONFIGURATION_MD: &str =
                    include_str!("../assets/docs/help_configuration.md");
                (
                    DocumentSource::BuiltIn(*self),
                    Some(String::from(HELP_CONFIGURATION_MD)),
                )
            }
            BuiltIn::Changelog => {
                const CHANGELOG_MD: &str = include_str!("../assets/docs/CHANGELOG.md");
                (
                    DocumentSource::BuiltIn(*self),
                    Some(String::from(CHANGELOG_MD)),
                )
            }
            BuiltIn::Welcome => (DocumentSource::BuiltIn(*self), None),
        }
    }

    pub fn relative_link(&self, link_url: &str) -> Option<(DocumentSource, Option<String>)> {
        match link_url {
            "./help_configuration.md" => Some(BuiltIn::HelpConfiguration.source()),
            _ => None,
        }
    }
}

impl TryFrom<&str> for BuiltIn {
    type Error = Error;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "help" => Ok(Self::Help),
            "help configuration" => Ok(Self::HelpConfiguration),
            "changelog" => Ok(Self::Changelog),
            "welcome" => Ok(Self::Welcome),
            _ => Err(Error::Navigation(NavigationError::UnknownLinkType(
                value.to_owned(),
            ))),
        }
    }
}

impl fmt::Display for BuiltIn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}",
            match self {
                BuiltIn::Help => "help",
                BuiltIn::HelpConfiguration => "help configuration",
                BuiltIn::Welcome => "welcome",
                BuiltIn::Changelog => "changelog",
            }
        )
    }
}

#[derive(Clone)]
pub struct SharedDocumentSource(pub Arc<RwLock<DocumentSource>>);

impl SharedDocumentSource {
    #[cfg(test)]
    pub fn test() -> SharedDocumentSource {
        SharedDocumentSource(Arc::new(RwLock::new(DocumentSource::Stdin { text: None })))
    }

    pub fn read(&self) -> Result<DocumentSource, Error> {
        Ok(self.0.read().map(|g| g.clone())?)
    }

    pub fn write(&self, source: DocumentSource) -> Result<(), Error> {
        let mut inner = self.0.write()?;
        *inner = source;
        Ok(())
    }
}

impl std::ops::Deref for SharedDocumentSource {
    type Target = RwLock<DocumentSource>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

pub fn open_source(
    source: &str,
    url_transform_command: Option<String>,
) -> Result<(String, DocumentSource), Error> {
    use ghrepo::GHRepo;
    use url::Url;

    use core::str::FromStr as _;
    if let Some(handle) = source.strip_prefix("github:")
        && let Ok(repo) = GHRepo::from_str(handle)
    {
        // We only want to try github if the user explicitly prefixed with "github:...".
        // Otherwise a path like "dir/file.md" would be a valid GHRepo and cause a useless request.
        let owner = repo.owner();
        let name = repo.name();
        let client = reqwest::blocking::Client::builder()
            .user_agent(format!(
                "mdfried/{}",
                VERSION.get().unwrap_or(&"unknown".to_owned())
            ))
            .build()?;
        for branch in ["master", "main"] {
            let url = format!(
                "https://raw.githubusercontent.com/{owner}/{name}/refs/heads/{branch}/README.md"
            );
            log::info!("trying github URL: {url}");
            print!("Fetching URL {url}...");
            let response = client.get(&url).send()?;
            if response.status().is_success() {
                println!("{OK_END}");
                return Ok((
                    response.text()?,
                    DocumentSource::Github {
                        repo,
                        branch: branch.to_owned(),
                    },
                ));
            } else {
                println!("error.");
            }
        }
        return Err(Error::Io(io::Error::other(format!(
            "failed to request https://raw.githubusercontent.com/{owner}/{name}/refs/heads/[master|main]/README.md"
        ))));
    } else if let Ok(url) = Url::parse(source) {
        log::info!("requesting URL: {url}");
        print!("Fetching URL {url}...");
        let client = reqwest::blocking::Client::builder()
            .user_agent(format!(
                "mdfried/{}",
                VERSION.get().unwrap_or(&"unknown".to_owned())
            ))
            .build()?;
        let response = client.get(url.as_ref()).send()?;
        if response.status().is_success() {
            println!("{OK_END}");
            log::debug!(
                "have url_transform_command? {}, content_type: {:?}",
                url_transform_command.is_some(),
                response.headers().get(CONTENT_TYPE)
            );
            if let Some(url_transform_command) = url_transform_command
                && let Some(content_type) = response.headers().get(CONTENT_TYPE)
                && content_type
                    .to_str()
                    .map_err(|err| {
                        Error::Io(io::Error::other(format!("content-type error: {err}")))
                    })?
                    .starts_with("text/html")
            {
                let mut child = Command::new("sh")
                    .arg("-c")
                    .arg(url_transform_command)
                    .env("URL", url.as_ref())
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .spawn()?;

                let Some(stdin) = child.stdin.as_mut() else {
                    return Err(Error::Io(io::Error::other(
                        "url_transform_command pipe error",
                    )));
                };
                stdin.write_all(response.text()?.as_bytes())?;

                let output = child.wait_with_output()?;

                return Ok((
                    String::from_utf8(output.stdout)
                        .map_err(|_err| Error::Io(io::Error::other("response not utf-8")))?,
                    DocumentSource::HyperText { url },
                ));
            }
            return Ok((response.text()?, DocumentSource::HyperText { url }));
        } else {
            println!("error.");
            return Err(Error::Io(io::Error::other(format!(
                "failed to request {url}"
            ))));
        }
    }

    let path = PathBuf::from(source);
    let basepath = path.parent().map(Path::to_path_buf);

    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "tiff" | "tif") => {
            return Ok((String::default(), DocumentSource::Image { path }));
        }
        Some("pdf") => {
            return Ok((String::default(), DocumentSource::Pdf { path }));
        }
        _ => {}
    }

    Ok((
        read_to_string(&path)?,
        DocumentSource::File { path, basepath },
    ))
}

pub fn github_usercontent_url(repo: &GHRepo, branch: &str, path: &str) -> Result<String, Error> {
    let path_url = Url::parse(&format!("https://dummy.com/{}", path))?; // :rolling_eyes:

    let mut url = Url::parse("https://raw.githubusercontent.com")?;
    let segs = path_url.path_segments().ok_or(Error::UrlParse(None))?;
    url.path_segments_mut()
        .or(Err(Error::UrlParse(None)))?
        .extend(&[repo.owner(), repo.name(), branch])
        .extend(segs);
    url.set_query(path_url.query());
    url.set_fragment(path_url.fragment());

    Ok(url.to_string())
}

pub fn extend_url(mut url: Url, path: &str) -> Result<String, Error> {
    let path_url = Url::parse(&format!("https://dummy.com/{}", path))?; // :rolling_eyes:

    let segs = path_url.path_segments().ok_or(Error::UrlParse(None))?;
    url.path_segments_mut()
        .or(Err(Error::UrlParse(None)))?
        .extend(segs);
    url.set_query(path_url.query());
    url.set_fragment(path_url.fragment());

    Ok(url.to_string())
}

pub struct DocumentHistoryEntry {
    pub source: DocumentSource,
    pub document: Document,
    pub scroll: u16,
}
