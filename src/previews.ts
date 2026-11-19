import type { StoredFile } from "./index"

const MIME_TYPES: Record<string, string> = {
  png: "image/png",
  jpg: "image/jpeg",
  jpeg: "image/jpeg",
  gif: "image/gif",
  webp: "image/webp",
  pdf: "application/pdf",
  txt: "text/plain",
  log: "text/plain",
  md: "text/markdown",
  csv: "text/csv",
  json: "application/json",
  yaml: "text/yaml",
  yml: "text/yaml",
  toml: "text/plain",
  js: "text/javascript",
  ts: "text/plain",
  py: "text/plain",
  rs: "text/plain",
  sh: "text/plain",
  html: "text/html",
  css: "text/css",
  xml: "application/xml",
  zip: "application/zip",
  mp4: "video/mp4",
  webm: "video/webm",
  mov: "video/quicktime",
}

export function contentType(filename: string, supplied = "application/octet-stream"): string {
  if (supplied !== "application/octet-stream") {
    return supplied
  }
  const extension = filename.slice(filename.lastIndexOf(".") + 1).toLowerCase()
  return MIME_TYPES[extension] ?? supplied
}

export function imageType(file: StoredFile): string | undefined {
  if (file.encrypted || file.once || file.size > 5 * 1024 * 1024) {
    return undefined
  }
  let type = contentType(file.filename, file.contentType)
  const parameterStart = type.indexOf(";")
  if (parameterStart !== -1) {
    type = type.slice(0, parameterStart)
  }
  type = type.trim()
  if (["image/png", "image/jpeg", "image/gif", "image/webp"].includes(type)) {
    return type
  }
  return undefined
}

export function isVideo(file: StoredFile): boolean {
  if (file.encrypted || file.once) {
    return false
  }
  const type = contentType(file.filename, file.contentType).split(";")[0].trim()
  return ["video/mp4", "video/webm", "video/quicktime"].includes(type)
}

export function isText(file: StoredFile): boolean {
  const type = contentType(file.filename, file.contentType)
  return type.startsWith("text/") || ["application/json", "application/xml"].includes(type)
}
