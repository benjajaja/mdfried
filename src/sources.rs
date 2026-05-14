use std::{
    fs::read_to_string,
    io::{self, Write as _},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use reqwest::header::CONTENT_TYPE;

use crate::{OK_END, VERSION, error::Error};

pub fn open_source(
    source: &str,
    url_transform_command: Option<String>,
) -> Result<(String, Option<PathBuf>, Option<PathBuf>), Error> {
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
                return Ok((response.text()?, None, None));
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
                    None,
                    None,
                ));
            }
            return Ok((response.text()?, None, None));
        } else {
            println!("error.");
            return Err(Error::Io(io::Error::other(format!(
                "failed to request {url}"
            ))));
        }
    }

    let path = PathBuf::from(source);
    let basepath = path.parent().map(Path::to_path_buf);
    Ok((read_to_string(&path)?, Some(path), basepath))
}
