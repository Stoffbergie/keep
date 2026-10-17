# keep

Upload a file. Get a link. Keep it until you delete it.

A small terminal tool with optional encryption and expiry. No account.

## Install

```sh
brew install stoffbergie/tap/keep
```

Already installed? Run `brew update && brew upgrade stoffbergie/tap/keep`.

## Share a file

```console
$ keep report.pdf
https://keep.stoff.dev/x7k2mzq4
```

Share several files with `keep report.pdf notes.txt`: each produces its own link. `--name` is for a single input. Keep checks every input before uploading; if a later transfer fails, earlier links remain usable.

Terminal uploads copy the links to your clipboard and show the filename, encryption and lifetime on stderr. Turn this off with `keep config --copy false`.

Large files show a compact progress bar immediately. Smaller uploads stay quiet for the first second. A finalizing spinner replaces the bar while the server finishes saving; the link appears when the file is ready. Keep the command running until it finishes.

Open the link in a browser, or download it from your terminal:

```sh
keep get x7k2mzq4
```

Use the short code or the full link. Downloads keep the original filename. In a pipe, `keep get` writes to stdout. Use `-o <file-or-directory>` to choose where to save it, or `-o -` to write to stdout explicitly.

## Share your clipboard

Copy text, an image or files, then run:

```sh
keep
```

Keep shows what it took with a short, gray preview. Text becomes `clipboard.txt`, images become `clipboard.png`, and copied files keep their names. Multiple files produce one link each; all links are copied together. Explicit files and piped input take priority over the clipboard.

## Share command output

```sh
make 2>&1 | keep - -n build.log
```

`-` explicitly reads stdin; omitting it still works when input is piped. Use it on its own, without file arguments.

`--name` (or `-n`) sets the download filename; it is optional. Output over 32 MB starts uploading while the command is still running. `|` sends output into Keep. `cat build.log > keep` writes a local file named `keep`; use `keep build.log` instead.

## Encrypt, list, delete

```sh
keep --encrypt .env
keep ls
keep rm x7k2mzq4
```

Encrypted links work in the browser and the CLI. Short encrypted links also work as `keep get 'x7k2mzq4#key'`. Keep the complete link: the part after `#` is the key and never reaches the server. Anyone with the complete link can decrypt the file. The filename is encrypted too: without the key, the page and the server only see `Encrypted file`.

Shared links show file previews. Encrypted files show an encrypted-file card. Deleting with `keep rm` requires the uploader's local history.

## Expire

```sh
keep --expires 1d .env
keep -e --expires 30m token.txt
keep -e --once recovery-codes.txt
keep -e --expires 1d --once recovery-codes.txt
```

`--expires` takes minutes, hours or days: `30m`, `12h`, `7d`. `--once` deletes the file after its first download; opening the page or a link preview doesn't count, and previews never show its contents. Combine them to delete after the first download or the deadline, whichever comes first. The older `--expires once` spelling still works. The link stops working once the time passes, and the file is deleted from storage within the hour. With curl, add `-H 'X-Expires: 1d'` and/or `-H 'X-Once: true'`; the response's `X-Expires-At` header says when. `keep ls` shows when each upload expires, or that it already has. Without either option, files stay until you delete them.

## Preferences

```sh
keep config                     # show preferences and their file
keep config --copy false         # stop copying uploaded links
keep config --clipboard false    # require a file or piped input
keep config --preview false      # hide clipboard content previews
```

Use `true` to enable any setting again. Encrypted uploads never preview clipboard text. `NO_COLOR` disables gray text. Redirected output stays plain and leaves the clipboard alone; a copy failure never fails an upload.

On Linux, copying links needs `wl-copy` (Wayland) or `xclip`/`xsel` (X11). Clipboard input needs a desktop session; Wayland also needs compositor support for data control. Files and pipes work without a clipboard.

## Use curl

```sh
curl -i -T report.pdf https://keep.stoff.dev/
curl -OJ https://keep.stoff.dev/x7k2mzq4
```

Keep the `X-Delete-Token` response header from a curl upload. To delete it, send `DELETE` to its link with `Authorization: Bearer <token>`. The CLI saves tokens for you and handles large files automatically. Single curl uploads are limited to 100 MB. Downloads support byte ranges, so `curl -C - -o report.pdf <link>` resumes an interrupted download.

## Develop or self-host

```sh
pnpm install
pnpm dev
```

The local Worker uses local storage. Build the CLI with `cargo install --path cli --locked`, then use `KEEP_URL=http://localhost:8787`. Run `pnpm check` and `pnpm test` for static checks and real upload/download workflows.

To self-host, sign in with `pnpm exec wrangler login`, create an R2 bucket with `pnpm exec wrangler r2 bucket create <bucket>`, and set the bucket and domain in `wrangler.toml`. Deploy with `pnpm deploy` and point `KEEP_URL` at your domain. GitHub Actions deploys `main`; version tags publish CLI releases and update Homebrew.

[Agent skill](SKILL.md) · [Releases](https://github.com/Stoffbergie/keep/releases) · [Website](https://keep.stoff.dev) · [MIT license](LICENSE)
