use anyhow::{Context, Result};
use clap::Args;
use serde::{Deserialize, Serialize};
use std::io::ErrorKind;
use std::path::PathBuf;

#[derive(Args)]
pub struct Options {
    #[arg(long, help = "Copy uploaded links to the clipboard")]
    copy: Option<bool>,
    #[arg(
        long,
        help = "Share the clipboard when no file or piped input is given"
    )]
    clipboard: Option<bool>,
    #[arg(long, help = "Show a short preview of clipboard contents")]
    preview: Option<bool>,
}

#[derive(Deserialize, Serialize)]
#[serde(default)]
pub struct Settings {
    pub copy: bool,
    pub clipboard: bool,
    pub preview: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            copy: true,
            clipboard: true,
            preview: true,
        }
    }
}

pub fn load() -> Result<Settings> {
    let path = path()?;
    match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .with_context(|| format!("could not read settings in {}", path.display())),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(Settings::default()),
        Err(error) => Err(error.into()),
    }
}

pub fn edit(options: Options) -> Result<()> {
    let mut settings = load()?;
    let path = path()?;
    if let Some(copy) = options.copy {
        settings.copy = copy;
    }
    if let Some(clipboard) = options.clipboard {
        settings.clipboard = clipboard;
    }
    if let Some(preview) = options.preview {
        settings.preview = preview;
    }
    if options.copy.is_some() || options.clipboard.is_some() || options.preview.is_some() {
        std::fs::create_dir_all(path.parent().context("settings directory is missing")?)?;
        let partial = path.with_extension("json.part");
        std::fs::write(&partial, serde_json::to_vec_pretty(&settings)?)?;
        std::fs::rename(partial, &path)?;
    }
    println!("copy: {}", settings.copy);
    println!("clipboard: {}", settings.clipboard);
    println!("preview: {}", settings.preview);
    println!("settings: {}", path.display());
    Ok(())
}

fn path() -> Result<PathBuf> {
    Ok(dirs::config_dir()
        .context("could not find a config directory")?
        .join("keep/settings.json"))
}
