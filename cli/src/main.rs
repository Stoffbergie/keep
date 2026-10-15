use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

mod clipboard;
mod config;
mod crypto;
mod download;
mod feedback;
mod history;
mod http;
mod upload;

#[derive(Parser)]
#[command(
    name = "keep",
    version,
    about = "Files from your terminal, kept forever",
    args_conflicts_with_subcommands = true,
    after_help = "Examples:\n  keep report.pdf                   Share a file\n  keep                              Share copied text, an image or files\n  keep --encrypt .env               Share an encrypted file\n  keep --expires 1d .env            Delete it after a day\n  make 2>&1 | keep --name build.log  Share command output\n  keep get x7k2mzq4                  Download by code or full link\n  keep config --copy false          Disable copying links\n\nUse | to pipe into keep; > writes to a file.\nSet KEEP_URL to use another server."
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    #[arg(help = "File to share; otherwise uses piped input or your clipboard")]
    file: Option<PathBuf>,

    #[arg(
        short,
        long,
        help = "Encrypt before uploading; the key stays in the link"
    )]
    encrypt: bool,

    #[arg(short, long, help = "Filename to store, defaults to the file's name")]
    name: Option<String>,

    #[arg(
        long,
        value_name = "TIME",
        help = "Delete the upload after this long, like 30m, 12h or 7d"
    )]
    expires: Option<String>,
}

#[derive(Subcommand)]
enum Command {
    #[command(about = "Download a file, decrypting it when the link has a key")]
    Get {
        #[arg(help = "Full link or code; include #key for encrypted files")]
        link: String,

        #[arg(short, long, help = "File or directory to write to, or - for stdout")]
        output: Option<String>,
    },
    #[command(about = "List uploads made from this machine")]
    Ls,
    #[command(about = "Delete an upload made from this machine")]
    Rm {
        #[arg(help = "Full link or code")]
        link: String,
    },
    #[command(about = "Show or change your preferences")]
    Config(config::Options),
}

fn main() {
    if let Err(error) = run(Cli::parse()) {
        eprintln!("keep: {error:#}");
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        match cli.command {
            Some(Command::Get { link, output }) => download::get(&link, output.as_deref()).await,
            Some(Command::Ls) => history::list(),
            Some(Command::Rm { link }) => history::remove(&link).await,
            Some(Command::Config(options)) => config::edit(options),
            None => {
                upload::upload(
                    cli.file.as_deref(),
                    cli.name,
                    cli.encrypt,
                    cli.expires.as_deref(),
                )
                .await
            }
        }
    })
}
