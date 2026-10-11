# Reference

## API

Uploads return the share link as plain text and a delete token in `X-Delete-Token`. Use `curl -i -T report.pdf https://keep.stoff.dev/` to include the response headers.

| Method | Path | Behavior |
| --- | --- | --- |
| `PUT` | `/:filename` | Upload, returning `201`, the share link and `X-Delete-Token`. Send `X-Encrypted: true` for encrypted bytes. |
| `GET` | `/:id` | Download bytes. Browsers and messaging crawlers receive the file page. |
| `GET` | `/:id/raw` | Download bytes regardless of the client. |
| `GET` | `/:id/preview` | Thumbnail for an unencrypted PNG, JPEG, GIF or WebP up to 5 MiB. |
| `DELETE` | `/:id` | Delete with `Authorization: Bearer <token>`. |
| `POST` | `/api/multipart?filename=` | Start an upload. Returns `{ id, uploadId }`. |
| `PUT` | `/api/multipart/:id/:part` | Upload a part with `X-Upload-Id`. Returns `{ partNumber, etag }`. |
| `POST` | `/api/multipart/:id` | Finish with `X-Upload-Id` and `{ "parts": [...] }`. Returns the share link and delete token. |
| `GET` | `/health` | Returns `{ "ok": true }`. |

```sh
curl -X DELETE -H "Authorization: Bearer $token" https://keep.stoff.dev/x7k2mzq4
```

Single requests are limited to 100 MB. The CLI uses four concurrent 32 MiB parts for files over 32 MiB, retries transient request failures and renames completed downloads into place.

## Clipboard and preferences

Uploads print their link to stdout and copy it to the clipboard when stdout is a terminal. The quiet confirmation goes to stderr. Redirected or piped output stays plain and leaves the clipboard alone. If copying fails, the upload still succeeds.

Use `keep config` to see your preferences and settings file. Run `keep config --copy false` to disable automatic copying, or `keep config --copy true` to restore it. `NO_COLOR` disables colored feedback.

On Linux, copying needs `wl-copy` on Wayland or `xclip`/`xsel` on X11. These helpers keep the link available after the command exits. macOS and Windows use their native clipboard.

## Storage and encryption

One Cloudflare Worker, one R2 bucket and no database. Each object stores its original filename, content type and delete token in metadata. Files remain until deleted with their token.

Encryption uses AES-256-GCM on the client. Stored bytes are the 12-byte nonce, ciphertext and 16-byte tag. The 32-byte key is base64url-encoded in the link fragment. The CLI and browser download page use the same format.

Public image links show thumbnails, text links show a short excerpt, and other files show their name and size. Encrypted previews show the filename and encrypted-file card. File pages and downloads ask search engines not to index them; the sitemap lists only the homepage.

## Self-host

You need Rust, Node.js, pnpm and a Cloudflare account with a domain.

1. Clone this repository and run `pnpm install`.
2. Sign in with `pnpm exec wrangler login`.
3. Create a bucket with `pnpm exec wrangler r2 bucket create keep-files`.
4. Set `bucket_name` and your domain in `wrangler.toml`.
5. Deploy with `pnpm deploy`.
6. Build the CLI with `cargo install --path cli --locked`.
7. Set `KEEP_URL=https://your.domain` when using the CLI.

## Develop

```sh
pnpm install
pnpm dev
pnpm check
pnpm test
```

The development server uses local storage at `http://localhost:8787`. To use the CLI against it, set `KEEP_URL=http://localhost:8787`.

`pnpm check` checks TypeScript, Rust formatting and Clippy. `pnpm test` runs real upload, download, encryption, deletion and link-preview workflows against a local Worker.

GitHub Actions deploys `main`. Version tags publish CLI releases and update the Homebrew formula.
