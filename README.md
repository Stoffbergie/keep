# keep

Upload a file. Get a link. Keep it until you delete it.

A small terminal tool with optional encryption. No account, no expiry.

## Install

```sh
brew install stoffbergie/tap/keep
```

## Share a file

```console
$ keep report.pdf
https://keep.stoff.dev/x7k2mzq4
```

Open the link in a browser, or download it from your terminal:

```sh
keep get https://keep.stoff.dev/x7k2mzq4
```

Downloads keep the original filename. In a pipe, `keep get` writes to stdout. Use `-o <file-or-directory>` to choose where to save it, or `-o -` to write to stdout explicitly.

## Everything else

```sh
keep -e .env                       # encrypt before uploading
cat build.log | keep -n build.log  # upload stdin with a filename
keep ls                           # uploads from this machine
keep rm <link>                    # delete an upload
```

Encrypted links work in the browser and the CLI. Keep the complete link: the part after `#` is the key and never reaches the server. Anyone with the complete link can decrypt the file. Filenames remain visible.

Shared links show file previews. Encrypted files show an encrypted-file card. Deleting an upload requires the local history saved by the machine that uploaded it.

## Use curl

```sh
curl -T report.pdf https://keep.stoff.dev/
curl -OJ https://keep.stoff.dev/x7k2mzq4
```

The CLI handles large files automatically. Single curl uploads are limited to 100 MB.

[API, self-hosting and development](docs/reference.md) · [Website](https://keep.stoff.dev) · [MIT license](LICENSE)
