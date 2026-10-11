use anyhow::Result;

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
