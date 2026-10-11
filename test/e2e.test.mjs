import assert from "node:assert/strict"
import { spawn, spawnSync } from "node:child_process"
import { createHash, randomBytes, webcrypto } from "node:crypto"
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs"
import { createServer } from "node:net"
import { tmpdir } from "node:os"
import { join, resolve } from "node:path"
import { after, before, test } from "node:test"
import { getPlatformProxy } from "wrangler"

const root = resolve(import.meta.dirname, "..")
const cliPath = join(root, "cli/target/debug/keep")
const wranglerPath = join(root, "node_modules/.bin/wrangler")

let work
let origin
let server
let serverLog = ""

function keep(args, input) {
  const result = spawnSync(cliPath, args, {
    cwd: work,
    input,
    encoding: "utf8",
    maxBuffer: 256 * 1024 * 1024,
    timeout: 120_000,
    env: { ...process.env, HOME: work, XDG_CONFIG_HOME: join(work, ".config"), KEEP_URL: origin },
  })
  return { status: result.status, stdout: result.stdout, stderr: result.stderr }
}

function upload(args, input) {
  const result = keep(args, input)
  assert.equal(result.status, 0, result.stderr)
  return result.stdout.trim()
}

function freePort() {
  return new Promise((resolvePort) => {
    const probe = createServer().listen(0, "127.0.0.1", () => {
      const { port } = probe.address()
      probe.close(() => resolvePort(port))
    })
  })
}

async function waitForServer() {
  const deadline = Date.now() + 60_000
  while (Date.now() < deadline) {
    try {
      const response = await fetch(`${origin}/health`)
      if (response.ok) {
        return
      }
    } catch {}
    await new Promise((resolveWait) => setTimeout(resolveWait, 250))
  }
  throw new Error(`wrangler dev did not start:\n${serverLog}`)
}

before(async () => {
  work = mkdtempSync(join(tmpdir(), "keep-e2e-"))
  const build = spawnSync("cargo", ["build", "--locked", "--manifest-path", join(root, "cli/Cargo.toml")], {
    encoding: "utf8",
  })
  assert.equal(build.status, 0, build.stderr)

  const state = join(work, "state")
  const { env, dispose } = await getPlatformProxy({
    configPath: join(root, "wrangler.toml"),
    persist: { path: join(state, "v3") },
    remoteBindings: false,
  })
  try {
    await env.FILES.put("old-file", "archived file\n", {
      customMetadata: {
        filename: "archive.txt",
        createdAt: "2026-01-25T10:00:00.000Z",
        deleteToken: "old-file-delete-token",
      },
    })
  } finally {
    await dispose()
  }
  const port = await freePort()
  origin = `http://127.0.0.1:${port}`
  const devArgs = [
    "dev",
    "--ip",
    "127.0.0.1",
    "--port",
    String(port),
    "--local-upstream",
    `127.0.0.1:${port}`,
    "--persist-to",
    state,
    "--show-interactive-dev-session=false",
  ]
  server = spawn(wranglerPath, devArgs, { cwd: root, env: { ...process.env, WRANGLER_SEND_METRICS: "false" } })
  server.stdout.on("data", (chunk) => (serverLog += chunk))
  server.stderr.on("data", (chunk) => (serverLog += chunk))
  await waitForServer()
})

after(() => {
  server?.kill()
  rmSync(work, { recursive: true, force: true })
})

test("uploads a file and downloads the same bytes under its name", async () => {
  const filename = "Dirk's résumé (final) #1.txt"
  writeFileSync(join(work, filename), "quarterly numbers\n")
  const link = upload([filename])
  assert.match(link, /^http:\/\/127\.0\.0\.1:\d+\/[0-9a-z]{8}$/)

  const headers = await fetch(`${link}/raw`, { method: "HEAD" })
  assert.equal(headers.status, 200)
  assert.equal(headers.headers.get("content-length"), String(Buffer.byteLength("quarterly numbers\n")))
  assert.equal((await headers.arrayBuffer()).byteLength, 0)

  const downloads = join(work, "downloads")
  mkdirSync(downloads)
  const saved = keep(["get", link, "-o", downloads])
  assert.equal(saved.status, 0, saved.stderr)
  assert.equal(saved.stdout.trim(), join(downloads, filename))
  assert.equal(readFileSync(join(downloads, filename), "utf8"), "quarterly numbers\n")
})

test("uploads stdin under a chosen name", async () => {
  const link = upload(["-n", "build.log"], "step 1 ok\nstep 2 ok\n")

  const fetched = keep(["get", link, "-o", "-"])
  assert.equal(fetched.stdout, "step 1 ok\nstep 2 ok\n")
  const response = await fetch(link)
  assert.match(response.headers.get("content-disposition"), /build\.log/)
})

test("saves clipboard preferences without changing scripted uploads", async () => {
  try {
    const changed = keep(["config", "--copy", "false", "--clipboard", "false", "--preview", "false"])
    assert.equal(changed.status, 0, changed.stderr)
    const current = keep(["config"])
    assert.match(current.stdout, /copy: false\nclipboard: false\npreview: false/)

    const link = upload(["--name", "piped.log"], "build passed\n")
    assert.equal(keep(["get", link, "-o", "-"]).stdout, "build passed\n")
  } finally {
    assert.equal(keep(["config", "--copy", "true", "--clipboard", "true", "--preview", "true"]).status, 0)
  }
})

test("encrypts on this machine and decrypts on download", async () => {
  writeFileSync(join(work, "secret.env"), "API_KEY=hunter2\n")
  const link = upload(["-e", "secret.env"])
  assert.match(link, /#[A-Za-z0-9_-]{43}$/)

  const stored = await (await fetch(link.split("#")[0])).text()
  assert.doesNotMatch(stored, /hunter2/)
  assert.equal(keep(["get", link, "-o", "-"]).stdout, "API_KEY=hunter2\n")
})

test("keeps encrypted files compatible with browser decryption and rejects changed ciphertext", async () => {
  const input = "private release notes\n"
  const link = upload(["-e", "-n", "release.env"], input)
  const [address, fragment] = link.split("#")
  const sealed = Buffer.from(await (await fetch(address)).arrayBuffer())
  const key = await webcrypto.subtle.importKey("raw", Buffer.from(fragment, "base64url"), "AES-GCM", false, ["decrypt"])
  const plain = await webcrypto.subtle.decrypt({ name: "AES-GCM", iv: sealed.subarray(0, 12) }, key, sealed.subarray(12))
  assert.equal(Buffer.from(plain).toString(), input)

  sealed[sealed.length - 1] ^= 1
  const changed = await fetch(`${origin}/changed.env`, { method: "PUT", body: sealed })
  const changedLink = `${(await changed.text()).trim()}#${fragment}`
  const destination = join(work, "changed-copy.env")
  const rejected = keep(["get", changedLink, "-o", destination])
  assert.equal(rejected.status, 1)
  assert.match(rejected.stderr, /could not decrypt/)
  assert.equal(existsSync(destination), false)
  assert.equal(keep(["get", changedLink, "-o", "-"]).stdout, "")

  const empty = upload(["-e", "-n", "empty.txt"], "")
  const downloaded = keep(["get", empty, "-o", "-"])
  assert.equal(downloaded.status, 0, downloaded.stderr)
  assert.equal(downloaded.stdout, "")
})

test("uploads large files in parts, downloads them intact and deletes them", () => {
  const payload = randomBytes(70 * 1024 * 1024)
  writeFileSync(join(work, "big.bin"), payload)
  const link = upload(["big.bin"])

  const saved = keep(["get", link, "-o", join(work, "big-copy.bin")])
  assert.equal(saved.status, 0, saved.stderr)
  const digest = (bytes) => createHash("sha256").update(bytes).digest("hex")
  assert.equal(digest(readFileSync(join(work, "big-copy.bin"))), digest(payload))
  const removed = keep(["rm", link])
  assert.equal(removed.status, 0, removed.stderr)
  const fetched = keep(["get", link, "-o", "-"])
  assert.equal(fetched.status, 1)
  assert.match(fetched.stderr, /Not found/)
  assert.doesNotMatch(keep(["ls"]).stdout, new RegExp(link))
})

test("deletes an upload and forgets it", () => {
  writeFileSync(join(work, "oops.txt"), "uploaded by mistake\n")
  const link = upload(["oops.txt"])
  assert.match(keep(["ls"]).stdout, new RegExp(link))

  const removed = keep(["rm", link])
  assert.equal(removed.status, 0, removed.stderr)
  const fetched = keep(["get", link, "-o", "-"])
  assert.equal(fetched.status, 1)
  assert.match(fetched.stderr, /Not found/)
  assert.doesNotMatch(keep(["ls"]).stdout, new RegExp(link))
})

test("works with plain curl", () => {
  const curl = (args) => spawnSync("curl", ["-fsS", ...args], { cwd: work, encoding: "utf8" })
  writeFileSync(join(work, "notes.md"), "# notes\n")
  const uploaded = curl(["-D", "headers.txt", "-T", "notes.md", origin])
  assert.equal(uploaded.status, 0, uploaded.stderr)
  const link = uploaded.stdout.trim()
  const token = readFileSync(join(work, "headers.txt"), "utf8").match(/x-delete-token: (\S+)/i)[1]

  const downloads = join(work, "curl-downloads")
  mkdirSync(downloads)
  assert.equal(spawnSync("curl", ["-fsSOJ", link], { cwd: downloads }).status, 0)
  assert.equal(readFileSync(join(downloads, "notes.md"), "utf8"), "# notes\n")

  assert.notEqual(curl(["-X", "DELETE", "-H", "Authorization: Bearer nope", link]).status, 0)
  assert.equal(curl(["-X", "DELETE", "-H", `Authorization: Bearer ${token}`, link]).status, 0)
  assert.notEqual(curl([link]).status, 0)
})

test("serves pages to browsers and usage to terminals", async () => {
  const html = { headers: { accept: "text/html" } }
  const home = await fetch(origin, html)
  assert.equal(home.status, 200)
  assert.match(await home.text(), /kept forever/)
  assert.match(await (await fetch(origin)).text(), /curl -T/)

  writeFileSync(join(work, "page.txt"), "page\n")
  const page = await fetch(upload(["page.txt"]), html)
  const pageBody = await page.text()
  assert.match(pageBody, /page\.txt/)
  assert.match(pageBody, /Download/)

  const encrypted = await fetch(upload(["-e", "page.txt"]).split("#")[0], html)
  assert.match(await encrypted.text(), /missing the key/)

  const missing = await fetch(`${origin}/nothing00`, html)
  assert.equal(missing.status, 404)
  assert.match(await missing.text(), /Nothing is kept at this address/)
})

test("shows the original upload date for migrated files and keeps them downloadable", async () => {
  const link = `${origin}/old-file`
  const page = await fetch(link, { headers: { accept: "text/html" } })
  assert.equal(page.status, 200)
  assert.match(await page.text(), /2026-01-25/)
  assert.equal(keep(["get", link, "-o", "-"]).stdout, "archived file\n")
  const removed = await fetch(link, {
    method: "DELETE",
    headers: { authorization: "Bearer old-file-delete-token" },
  })
  assert.equal(removed.status, 204)
  assert.equal((await fetch(link)).status, 404)
})

test("unfurls the homepage and public text files without changing downloads", async () => {
  const crawler = { headers: { "user-agent": "WhatsApp/2.24.1", accept: "*/*" } }
  const home = await fetch(origin, crawler)
  assert.match(home.headers.get("content-type"), /text\/html/)
  const body = await home.text()
  assert.match(body, /name="description"/)
  const image = body.match(/property="og:image" content="([^"]+)"/)[1]
  const imageResponse = await fetch(image, crawler)
  assert.equal(imageResponse.status, 200)
  assert.equal(imageResponse.headers.get("content-type"), "image/png")
  assert.deepEqual(Buffer.from(await imageResponse.arrayBuffer()), readFileSync(join(root, "public/social/keep.png")))

  const filename = "Résumé & release notes.txt"
  const link = upload(["-n", filename], "Build complete. All checks passed.\n")
  const page = await (await fetch(link, crawler)).text()
  assert.match(page, /Résumé &amp; release notes\.txt · keep/)
  assert.match(page, /Build complete\. All checks passed\./)
  assert.match(page, /name="robots" content="noindex, nofollow"/)
  assert.equal(keep(["get", link, "-o", "-"]).stdout, "Build complete. All checks passed.\n")
  const robots = await (await fetch(`${origin}/robots.txt`)).text()
  assert.match(robots, /Sitemap:/)
  const sitemap = await (await fetch(`${origin}/sitemap.xml`)).text()
  assert.match(sitemap, new RegExp(`<loc>${origin}/</loc>`))
  assert.doesNotMatch(sitemap, new RegExp(link))
})

test("shares an image thumbnail with its filename and hides encrypted image contents", async () => {
  const crawler = { headers: { "user-agent": "facebookexternalhit/1.1", accept: "*/*" } }
  const filename = "Holiday photo é.png"
  const bytes = readFileSync(join(root, "public/social/file.png"))
  writeFileSync(join(work, filename), bytes)
  const link = upload([filename])
  const page = await (await fetch(link, crawler)).text()
  assert.match(page, /Holiday photo é\.png · keep/)
  const image = page.match(/property="og:image" content="([^"]+)"/)[1]
  const preview = await fetch(image, crawler)
  assert.equal(preview.headers.get("content-type"), "image/png")
  assert.match(preview.headers.get("content-disposition"), /^inline/)
  assert.deepEqual(Buffer.from(await preview.arrayBuffer()), bytes)

  const encrypted = upload(["-e", filename])
  const sealedPage = await (await fetch(encrypted.split("#")[0], crawler)).text()
  assert.match(sealedPage, /Holiday photo é\.png · encrypted · keep/)
  assert.doesNotMatch(sealedPage, new RegExp(encrypted.split("#")[1]))
  const sealedImage = sealedPage.match(/property="og:image" content="([^"]+)"/)[1]
  const card = await fetch(sealedImage, crawler)
  assert.deepEqual(Buffer.from(await card.arrayBuffer()), readFileSync(join(root, "public/social/encrypted.png")))
  assert.equal((await fetch(`${encrypted.split("#")[0]}/preview`, crawler)).status, 404)

  const downloads = join(work, "encrypted-downloads")
  mkdirSync(downloads)
  const saved = keep(["get", encrypted, "-o", downloads])
  assert.equal(saved.status, 0, saved.stderr)
  assert.deepEqual(readFileSync(join(downloads, filename)), bytes)
  assert.equal(keep(["rm", link]).status, 0)
  assert.equal((await fetch(image, crawler)).status, 404)
})
