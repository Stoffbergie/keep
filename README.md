<p align="center">
  <img src="public/apple-touch-icon.png" width="72" height="72" alt="" />
</p>

<h1 align="center">keep</h1>

<p align="center">
  <strong>Upload a file. Share a link.</strong><br />
  Files, screenshots and command output, straight from your terminal.
</p>

<p align="center">
  <a href="https://github.com/Stoffbergie/keep/releases"><img src="https://img.shields.io/github/v/release/Stoffbergie/keep?color=171717&label=release" alt="Latest release" /></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-171717" alt="MIT license" /></a>
</p>

<p align="center">
  <a href="#start-sharing">Get started</a> ·
  <a href="#everyday-commands">Commands</a> ·
  <a href="#share-something-private">Encryption</a> ·
  <a href="https://keep.stoff.dev">Website</a> ·
  <a href="SKILL.md">For agents</a>
</p>

---

## Start sharing

Install with Homebrew:

```sh
brew install stoffbergie/tap/keep
```

Then share a file:

```console
$ keep report.pdf
https://keep.stoff.dev/x7k2mzq4
```

The link is copied to your clipboard. Use `keep --no-copy report.pdf` to leave your clipboard alone for this upload. Send the link to someone; they can download in their browser without installing Keep.

**Files stay until you delete them.** Add encryption, an expiry, or a one-download limit when you need it. No account to create.

<details>
<summary>Other downloads and updates</summary>

Download a binary for macOS, Linux or Windows from [Releases](https://github.com/Stoffbergie/keep/releases).

The Linux GNU downloads need glibc 2.34 or newer on x86_64, and glibc 2.39 or newer on ARM64. The x86_64 GNU build may print a glibc 2.39 warning on older systems. Choose `keep-x86_64-unknown-linux-musl.tar.gz` for x86_64 Linux without glibc or with an older version. There is no ARM64 musl download.

Already installed with Homebrew?

```sh
brew update && brew upgrade stoffbergie/tap/keep
```

</details>

## Everyday commands

| What you want to share | Command |
| --- | --- |
| A file | `keep report.pdf` |
| Several files, one link each | `keep report.pdf notes.txt` |
| Copied text, an image or files | `keep` |
| Command output | `make 2>&1 \| keep - -n build.log` |
| A file with a different download name | `keep report.pdf -n annual-report.pdf` |

Links go to stdout; progress and file details go to stderr. Large uploads show estimated transfer time remaining, followed by “Waiting for server” and “Saving file”. The link appears as soon as the file is ready. Small uploads skip the progress display if they finish within a second.

## Share something private

```sh
keep -e --expires 1d --once secrets.txt
```

**Encrypted before it leaves your machine.** Both the file and its name are encrypted. The key lives after `#` in the link and never reaches the server. The browser and CLI decrypt locally.

**Gone after one download or one day.** Whichever happens first. Opening the page or a link preview doesn't count as a download, and previews never reveal the contents of a one-time upload.

Each option also works on its own:

| Option | What it does |
| --- | --- |
| `-e` / `--encrypt` | Encrypt the file and filename |
| `--expires 30m` | Expire after minutes, hours or days: `30m`, `12h`, `7d` |
| `--once` | Delete after the first download |

Keep the **complete link**, including its key. Anyone with that link can decrypt the file; without the key, the page only shows “Encrypted file”.

## Get it back. Take it down.

Download with a short code or a full link:

```sh
keep get x7k2mzq4
```

The file saves with its original name. For encrypted files, use the complete link in quotes, including `#key`.

See your uploads, then delete the ones you're done sharing:

```sh
keep ls
keep rm x7k2mzq4 b8p4ntw6
```

History and delete tokens stay on the machine that uploaded the files. `keep ls` also shows encryption, expiry and one-download limits.

## A little more control

<details>
<summary><strong>Clipboard and piped input</strong></summary>

Run `keep` after copying text, an image or files. Text becomes `clipboard.txt`, images become `clipboard.png`, and files keep their names. Keep shows a short, gray preview of what it took; encrypted uploads never preview clipboard text. When sharing several files, all their links are copied together.

Explicit files take priority over piped input, then the clipboard. `-` explicitly reads stdin; it must be used on its own, without file arguments. Omitting `-` still works when input is piped. Output over 32 MB starts uploading while the command is still running; piped input has no time estimate while its total size is unknown.

`--name` (or `-n`) sets the download filename for one input. Keep checks every input before uploading. If a later transfer fails, earlier links remain usable.

Use `|` to pipe into Keep. `cat build.log > keep` writes a local file named `keep`; use `keep build.log` instead.

</details>

<details>
<summary><strong>Download destinations and scripts</strong></summary>

| Save to | Command |
| --- | --- |
| A chosen filename | `keep get x7k2mzq4 -o report.pdf` |
| An existing directory | `keep get x7k2mzq4 -o ./downloads/` |
| Standard output | `keep get x7k2mzq4 -o -` |

`keep get` always saves a file and prints its path, even when stdout is captured. Scripts that relied on the previous piped-output default must add `-o -`. Short encrypted links work too: `keep get 'x7k2mzq4#key'`.

Keep validates the key and destination before fetching contents, so a missing key or an unwritable path does not consume a one-time upload.

Agents and scripts should pass `--no-copy` on every upload, including when running in a terminal or pseudo-terminal. It preserves the clipboard without changing saved preferences. Captured upload output contains one link per successful file and also leaves the clipboard alone. Check the exit status before using those links. See the [agent guide](SKILL.md) for automation examples.

</details>

<details>
<summary><strong>Preferences and Linux clipboard support</strong></summary>

Run `keep config` to see your preferences and their file. Change any setting with `true` or `false`:

| Preference | Command |
| --- | --- |
| Stop copying uploaded links | `keep config --copy false` |
| Require a file or piped input | `keep config --clipboard false` |
| Hide clipboard content previews | `keep config --preview false` |

`NO_COLOR` disables gray text. Redirected output stays plain, and a clipboard copy failure never fails an upload.

On Linux, copying links needs `wl-copy` on Wayland, or `xclip` / `xsel` on X11. Clipboard input needs a desktop session; Wayland also needs compositor support for data control. Files and pipes work without a clipboard.

</details>

<details>
<summary><strong>Use curl, no Keep installation needed</strong></summary>

Upload a file, or download an unencrypted one with its original filename:

```sh
curl -i -T report.pdf https://keep.stoff.dev/
curl -fLOJ https://keep.stoff.dev/x7k2mzq4/raw
```

Keep the `X-Delete-Token` response header from an upload. To delete it, send `DELETE` to its link with `Authorization: Bearer <token>`. The CLI saves tokens for you and handles large files automatically; single curl uploads are limited to 100 MB.

Downloads support byte ranges: `curl -C - -o report.pdf <link>` resumes an interrupted download. Encrypted downloads need Keep or a browser to decrypt; `/raw` returns encrypted bytes.

For an expiry, add `-H 'X-Expires: 1d'` and/or `-H 'X-Once: true'`. The response's `X-Expires-At` header gives the deadline. Expired links stop serving immediately; expired files are removed from storage within the hour. The older `keep --expires once` spelling still works.

</details>

<details>
<summary><strong>Develop or self-host</strong></summary>

One Rust CLI, one Cloudflare Worker, one R2 bucket.

```sh
pnpm install
pnpm dev
```

The local Worker uses local storage. Build the CLI with `cargo install --path cli --locked`, then set `KEEP_URL=http://localhost:8787` to use it. Run `pnpm check` and `pnpm test` for static checks and real upload/download workflows.

To self-host, sign in with `pnpm exec wrangler login`, create an R2 bucket with `pnpm exec wrangler r2 bucket create <bucket>`, and set the bucket and domain in `wrangler.toml`. Deploy with `pnpm deploy` and point `KEEP_URL` at your domain.

GitHub Actions deploys `main`; version tags publish CLI releases and update Homebrew.

</details>

---

<p align="center">
  <a href="https://keep.stoff.dev">keep.stoff.dev</a> ·
  <a href="https://github.com/Stoffbergie/keep/releases">Releases</a> ·
  <a href="LICENSE">MIT licensed</a>
</p>
