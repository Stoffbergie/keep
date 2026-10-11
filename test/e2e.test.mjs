import assert from "node:assert/strict"
import { spawn, spawnSync } from "node:child_process"
import { createHash, randomBytes } from "node:crypto"
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs"
import { createServer } from "node:net"
import { tmpdir } from "node:os"
import { join, resolve } from "node:path"
import { after, before, test } from "node:test"

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

test("uploads a file and downloads the same bytes under its name", () => {
  writeFileSync(join(work, "report.txt"), "quarterly numbers\n")
  const link = upload(["report.txt"])
  assert.match(link, /^http:\/\/127\.0\.0\.1:\d+\/[0-9a-z]{8}$/)

  const downloads = join(work, "downloads")
  mkdirSync(downloads)
  const saved = keep(["get", link, "-o", downloads])
  assert.equal(saved.status, 0, saved.stderr)
  assert.equal(saved.stdout.trim(), join(downloads, "report.txt"))
  assert.equal(readFileSync(join(downloads, "report.txt"), "utf8"), "quarterly numbers\n")
})

test("uploads stdin under a chosen name", async () => {
  const link = upload(["-n", "build.log"], "step 1 ok\nstep 2 ok\n")

  const fetched = keep(["get", link, "-o", "-"])
  assert.equal(fetched.stdout, "step 1 ok\nstep 2 ok\n")
  const response = await fetch(link)
  assert.match(response.headers.get("content-disposition"), /build\.log/)
})

test("encrypts on this machine and decrypts on download", async () => {
  writeFileSync(join(work, "secret.env"), "API_KEY=hunter2\n")
  const link = upload(["-e", "secret.env"])
  assert.match(link, /#[A-Za-z0-9_-]{43}$/)

  const stored = await (await fetch(link.split("#")[0])).text()
  assert.doesNotMatch(stored, /hunter2/)
  assert.equal(keep(["get", link, "-o", "-"]).stdout, "API_KEY=hunter2\n")
})

test("uploads large files in parts and downloads them intact", () => {
  const payload = randomBytes(70 * 1024 * 1024)
  writeFileSync(join(work, "big.bin"), payload)
  const link = upload(["big.bin"])

  const saved = keep(["get", link, "-o", join(work, "big-copy.bin")])
  assert.equal(saved.status, 0, saved.stderr)
  const digest = (bytes) => createHash("sha256").update(bytes).digest("hex")
  assert.equal(digest(readFileSync(join(work, "big-copy.bin"))), digest(payload))
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
