# keep

Upload a file. Get a link. Keep it until you delete it.

A small terminal tool with optional encryption. No account, no expiry.

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

Terminal uploads copy the link to your clipboard. Turn this off with `keep config --copy false`.

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
make 2>&1 | keep --name build.log
```

`--name` (or `-n`) sets the download filename; it is optional. `|` sends output into Keep. `cat build.log > keep` writes a local file named `keep`; use `keep build.log` instead.

## Encrypt, list, delete

```sh
keep --encrypt .env
keep ls
keep rm x7k2mzq4
```

Encrypted links work in the browser and the CLI. Short encrypted links also work as `keep get 'x7k2mzq4#key'`. Keep the complete link: the part after `#` is the key and never reaches the server. Anyone with the complete link can decrypt the file. Filenames remain visible.

Shared links show file previews. Encrypted files show an encrypted-file card. Deleting with `keep rm` requires the uploader's local history.

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

Keep the `X-Delete-Token` response header from a curl upload. To delete it, send `DELETE` to its link with `Authorization: Bearer <token>`. The CLI saves tokens for you and handles large files automatically. Single curl uploads are limited to 100 MB.

## Develop or self-host

```sh
pnpm install
pnpm dev
```

The local Worker uses local storage. Build the CLI with `cargo install --path cli --locked`, then use `KEEP_URL=http://localhost:8787`. Run `pnpm check` and `pnpm test` for static checks and real upload/download workflows.

To self-host, sign in with `pnpm exec wrangler login`, create an R2 bucket with `pnpm exec wrangler r2 bucket create <bucket>`, and set the bucket and domain in `wrangler.toml`. Deploy with `pnpm deploy` and point `KEEP_URL` at your domain. GitHub Actions deploys `main`; version tags publish CLI releases and update Homebrew.

[Agent skill](SKILL.md) · [Releases](https://github.com/Stoffbergie/keep/releases) · [Website](https://keep.stoff.dev) · [MIT license](LICENSE)
