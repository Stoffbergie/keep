use anyhow::{bail, Context, Result};
use std::path::PathBuf;

pub enum Contents {
    Files(Vec<PathBuf>),
    Image {
        png: Vec<u8>,
        width: u32,
        height: u32,
    },
    Text(String),
}

pub fn read() -> Result<Contents> {
    let mut clipboard = arboard::Clipboard::new().context("could not open the clipboard")?;
    if let Ok(files) = clipboard.get().file_list() {
        if !files.is_empty() {
            return Ok(Contents::Files(files));
        }
    }
    if let Ok(image) = clipboard.get_image() {
        let width = image.width.try_into()?;
        let height = image.height.try_into()?;
        let mut png = Vec::new();
        let mut encoder = png::Encoder::new(&mut png, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Fast);
        let mut writer = encoder.write_header()?;
        writer.write_image_data(&image.bytes)?;
        writer.finish()?;
        return Ok(Contents::Image { png, width, height });
    }
    if let Ok(text) = clipboard.get_text() {
        if !text.is_empty() {
            return Ok(Contents::Text(text));
        }
    }
    bail!("clipboard has no text, image or files; pass a file or pipe into keep")
}

#[cfg(not(target_os = "linux"))]
pub async fn copy(text: &str) -> Result<()> {
    arboard::Clipboard::new()?.set_text(text)?;
    Ok(())
}

#[cfg(target_os = "linux")]
pub async fn copy(text: &str) -> Result<()> {
    use anyhow::bail;
    use std::process::Stdio;
    use std::time::Duration;
    use tokio::io::AsyncWriteExt;
    use tokio::process::Command;

    let commands: &[(&str, &[&str])] = if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        &[("wl-copy", &[])]
    } else if std::env::var_os("DISPLAY").is_some() {
        &[
            ("xclip", &["-selection", "clipboard"]),
            ("xsel", &["--clipboard", "--input"]),
        ]
    } else {
        bail!("no desktop clipboard is available");
    };
    tokio::time::timeout(Duration::from_secs(2), async {
        for (program, args) in commands {
            let child = Command::new(program)
                .args(*args)
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .kill_on_drop(true)
                .spawn();
            let Ok(mut child) = child else { continue };
            if let Some(mut input) = child.stdin.take() {
                input.write_all(text.as_bytes()).await?;
            }
            if child.wait().await?.success() {
                return Ok(());
            }
        }
        bail!("install wl-clipboard, xclip or xsel to copy links");
    })
    .await?
}
