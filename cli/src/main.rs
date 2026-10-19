use anyhow::{bail, Result};
use clap::{Parser, Subcommand};
use futures_util::future::{select, Either};
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
    about = "Share files from your terminal with a link.",
    args_conflicts_with_subcommands = true,
    after_help = "Examples:\n  keep report.pdf notes.txt        Share files, one link each\n  keep                             Share copied text, an image or files\n  keep -e --expires 1d --once .env Encrypt; delete after a day or one download\n  make 2>&1 | keep - -n build.log  Share command output\n  keep get x7k2mzq4                Save with its original filename\n  keep get x7k2mzq4 -o -           Write file contents to stdout\n  keep rm x7k2mzq4 b8p4ntw6        Delete uploads from this machine\n  keep --no-copy report.pdf        Upload without changing the clipboard\n  keep config --copy false         Disable copying links by default\n\nUse | to pipe into keep; > writes to a file.\n--expires once still works; --once also combines with a deadline.\nOpening a page or link preview does not count as a download.\nSet KEEP_URL to use another server."
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    #[arg(help = "Files to share, or - for stdin; otherwise uses piped input or your clipboard")]
    files: Vec<PathBuf>,

    #[arg(
        short,
        long,
        help = "Encrypt before uploading; the key stays in the link"
    )]
    encrypt: bool,

    #[arg(
        short,
        long,
        help = "Download filename for one input; defaults to its original name"
    )]
    name: Option<String>,

    #[arg(
        long,
        value_name = "TIME",
        help = "Delete after this long, like 30m, 12h or 7d"
    )]
    expires: Option<String>,

    #[arg(
        long,
        help = "Delete after the first download; combine with --expires for a deadline"
    )]
    once: bool,

    #[arg(long, help = "Leave the clipboard unchanged after uploading")]
    no_copy: bool,
}

#[derive(Subcommand)]
enum Command {
    #[command(
        about = "Save a file under its original name, decrypting it when the link has a key"
    )]
    Get {
        #[arg(help = "Full link or code; include #key for encrypted files")]
        link: String,

        #[arg(
            short,
            long,
            help = "File or existing directory to save in; - writes contents to stdout"
        )]
        output: Option<String>,
    },
    #[command(about = "List uploads made from this machine")]
    Ls,
    #[command(about = "Delete uploads made from this machine")]
    Rm {
        #[arg(required = true, help = "Full links or codes")]
        links: Vec<String>,
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
    match cli.command {
        Some(Command::Get { link, output }) => run_async(download::get(&link, output.as_deref())),
        Some(Command::Ls) => history::list(),
        Some(Command::Rm { links }) => run_async(history::remove(&links)),
        Some(Command::Config(options)) => config::edit(options),
        None => run_async(async {
            let lifetime = upload::Lifetime::new(cli.expires, cli.once)?;
            let _cursor = feedback::HiddenCursor::new();
            let upload = upload::upload(&cli.files, cli.name, cli.encrypt, &lifetime, cli.no_copy);
            let result = match select(Box::pin(tokio::signal::ctrl_c()), Box::pin(upload)).await {
                Either::Left((signal, _)) => {
                    signal?;
                    bail!("upload cancelled");
                }
                Either::Right((result, _)) => result,
            };
            result
        }),
    }
}

fn run_async(task: impl std::future::Future<Output = Result<()>>) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(task)
}
