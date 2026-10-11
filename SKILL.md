---
name: keep
description: Shares and retrieves files through the keep CLI, including encrypted links, clipboard sharing, cross-machine transfers, and deleting uploads. Use when a request involves keep.stoff.dev, a Keep link or code, or transferring a file with Keep. Not for developing, deploying, or releasing the keep repository.
---

# Keep

Produce a usable share link or a verified local download. Files remain on the chosen server until deleted; there are no accounts or expiry settings.

## Choose the input and destination

- Use the file, output, clipboard contents, server and recipient authorized by the request. Resolve missing choices from context; ask only when they change what will be shared or where it goes.
- Prefer an explicit file or pipe in agent runs. Bare `keep` reads the clipboard only when its input is a terminal; use it only for an intentional clipboard-sharing request. Files take priority over pipes, then the clipboard.
- Use `--encrypt` for private contents unless the user requires a public file. The full link contains the decryption key: share it only with the intended audience. Filenames remain public, so use `--name` when a sensitive name also needs replacing.
- Check `command -v keep` and `keep --help`. If absent, install with `brew install stoffbergie/tap/keep` where Homebrew is available, or use a matching binary from [official releases](https://github.com/Stoffbergie/keep/releases). Do not change global preferences to complete a one-off task.
- The default server is `https://keep.stoff.dev`. Preserve an existing `KEEP_URL`; set it per command when the user requests another server. Short codes use that server, while full links specify their own.

## Share

```sh
keep ./report.pdf
keep --encrypt ./private.env
make 2>&1 | keep --name build.log
```

`--name` (`-n`) changes the stored download filename; it does not create a custom URL. Archive a directory before uploading it. A pipe uploads its actual bytes, including empty input; it cannot infer the original filename.

Capture the returned link when another command needs it:

```sh
url=$(keep ./report.pdf)
```

Check the exit status before using the result. Stdout contains one link per successful upload; feedback goes to stderr. Terminal uploads copy links by default. Captured or redirected output leaves the clipboard alone, so command substitution is suitable for automation.

For intentional clipboard sharing, run `keep` in a terminal. Copied files retain their names; text becomes `clipboard.txt` and images become `clipboard.png`. Several files produce several links, copied together. Do not assume clipboard access in a headless runner or synthesize an empty pipe to request it.

## Retrieve

Always choose an output destination in an agent run. Without `--output`, non-terminal downloads write bytes to stdout, including binary files.

```sh
keep get "$url" --output ./downloads/
keep get x7k2mzq4 --output ./report.pdf
keep get 'x7k2mzq4#key' --output ./private.env
```

The last example uses a placeholder: retain the real fragment from the supplied encrypted link. A code alone cannot decrypt an encrypted file. Quote complete links and never drop or expose their keys in logs.

An existing output directory preserves the stored filename. An explicit output stops if the destination file already exists; choose a fresh path or resolve the collision with the user. Interactive downloads without an output option choose a free filename. Use `--output -` only when the next command intentionally consumes the bytes.

For a cross-machine transfer, upload on the source, download on the destination and compare SHA-256 hashes on both machines. Use the available checksum tool, such as `shasum -a 256` or `sha256sum`. A returned link or successful HTTP response alone does not prove the transferred bytes match. Downloaded files appear at their final path only after completion.

## List and delete

```sh
keep ls
keep rm x7k2mzq4
```

Listing and deletion use local upload history, which contains full links and delete tokens. Use the uploading environment's history; a share link alone is insufficient to delete a file. Delete only uploads covered by the request, including disposable verification uploads. Do not print unrelated history or tokens.

## Verify and recover

- For a requested round trip, download to a new path and compare with `cmp` or matching hashes. For encryption, verify the decrypted bytes; fetching ciphertext is insufficient.
- If an upload reports a history-save failure, the file is already uploaded. Preserve its returned link and explain that local listing/deletion was not saved. Do not upload again merely to repair history.
- If clipboard copying fails, use the printed link; the upload still succeeded. If clipboard input is unavailable or disabled, use the intended file or pipe instead. Inspect `keep config` only when diagnosing preferences.
- If a batch stops partway, retain every successful link and retry only the remaining files. Keep already retries transient network failures; avoid blind loops that create duplicate permanent uploads.
- If download fails, do not use a partial file as a completed result. A missing key needs the original complete link; a missing file needs the correct link/server or a new upload from its owner.

Report the resulting link or local path, what was actually verified, and any remaining blocker. Do not claim recipient access, browser decryption or a remote transfer was tested unless it was.
