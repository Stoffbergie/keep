Contract notes for this repo:

- keep is a Rust CLI, one Cloudflare Worker and one R2 bucket. Keep it that way: no accounts and no database. Link previews show public file details or an encrypted-file card.
- Files are permanent until deleted with their delete token, unless uploaded with `--expires` (`X-Expires`). Expired files stop being served at once and an hourly cron deletes them from R2. `--expires once` files are deleted after their first content download; pages, HEAD requests and link previews don't count and never show their contents.
- The CLI surface is `keep [file]`, `keep get`, `keep ls`, `keep rm` and `keep config`, with `-e`, `-n`, `--expires` and `-o`. Explicit files take priority over piped input, then the clipboard. Terminal uploads copy links by default; captured stdout stays links only and never changes the clipboard. Add a flag only when a workflow cannot be done without it.
- Encryption happens on the client with AES-256-GCM and the base64url key in the link fragment. Files are streamed as `keep-v2\n`, a 7-byte nonce prefix, then 1 MiB chunks of ciphertext and tag, each with nonce prefix, 4-byte big-endian chunk index and a final-chunk byte. Older files and filenames are a single nonce, ciphertext, tag. Encrypted uploads seal the filename that way, base64url-encoded in `X-Encrypted-Name`, and store `encrypted` as the visible name. The server never sees keys, and the CLI and the download page must stay compatible.
- Uploads respond with the link as text plus `X-Delete-Token`, so `curl -T` stays useful.
- Each IP gets 20 Not found file lookups a minute; after that every lookup from it gets 429 for a minute, so codes can't be guessed.
- Pages are monospace, monotone and framework-free, with system fonts only.
- Tests are end-to-end in `test/e2e.test.mjs`. Cover user workflows, not internals.
