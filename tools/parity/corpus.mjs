// Builds every project under crates/leafcutter-ant/tests/corpus with TS Dash
// 0.13.0 and writes what each build changed to <project>.expected.json, which
// the Rust test crates/leafcutter-ant/tests/corpus.rs compares against.
//
// Each project is built four times, the ways the Deno CLI can build it:
// `production` and `development` write into the project, and the `-out`
// variants give Dash a separate output file system, as `--out` does. Every
// build starts from a fresh copy of the project in a temporary directory.
//
// The host here plays the Deno CLI's part with two changes that fix what the
// Deno CLI leaves to timing: directories are listed sorted by the bytes of
// their UTF-8 names, and `allFiles` finishes in pack order. leafcutter-ant
// does the same; see the README's differences table.
//
// Usage: npm ci && npm run vectors   (from tools/parity)

import { cpSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs"
import { createRequire } from "node:module"
import { tmpdir } from "node:os"
import { dirname, isAbsolute, join as nodeJoin } from "node:path"
import { fileURLToPath } from "node:url"
import { isMatch } from "@bridge-editor/common-utils"
import { Console, Dash, FileSystem, initRuntimes } from "@bridge-editor/dash-compiler"
import { FileType, PackType } from "@bridge-editor/mc-project-core"
import { join } from "pathe"

const require = createRequire(import.meta.url)
const corpusDir = fileURLToPath(new URL("../../crates/leafcutter-ant/tests/corpus/", import.meta.url))
const fileDefinitions = require("../../crates/leafcutter-ant/tests/data/fileDefinitions.json")
const packDefinitions = require("../../crates/leafcutter-ant/tests/data/packDefinitions.json")

initRuntimes(readFileSync(require.resolve("@swc/wasm-web/wasm-web_bg.wasm")))

// floatPropertyTruncationFix logs through the global console.
console.log = () => {}

const byUtf8 = (a, b) => Buffer.compare(Buffer.from(a.name), Buffer.from(b.name))

// The Deno CLI's DenoFileSystem, on Node's synchronous calls.
class NodeFileSystem extends FileSystem {
	constructor(base) {
		super()
		this.base = base
	}
	resolve(path) {
		return isAbsolute(path) ? path : nodeJoin(this.base, path)
	}
	async readFile(path) {
		return new File([readFileSync(this.resolve(path))], path.split("/").pop())
	}
	async writeFile(path, content) {
		mkdirSync(dirname(this.resolve(path)), { recursive: true })
		writeFileSync(this.resolve(path), content)
	}
	async unlink(path) {
		rmSync(this.resolve(path), { recursive: true })
	}
	async readdir(path) {
		return readdirSync(this.resolve(path), { withFileTypes: true })
			.map((entry) => ({ name: entry.name, kind: entry.isDirectory() ? "directory" : "file" }))
			.sort(byUtf8)
	}
	// FileSystem.allFiles, with the directory reads made synchronous so that
	// every pack's listing finishes in the same number of steps.
	async allFiles(path) {
		const walk = (dir) => {
			const files = []
			for (const entry of readdirSync(this.resolve(dir), { withFileTypes: true }).sort(byUtf8)) {
				if (entry.isDirectory()) files.push(...walk(join(dir, entry.name)))
				else files.push(join(dir, entry.name))
			}
			return files
		}
		return walk(path)
	}
	async mkdir(path) {
		mkdirSync(this.resolve(path), { recursive: true })
	}
	async lastModified(path) {
		return statSync(this.resolve(path)).mtimeMs
	}
}

// The Deno CLI's FileTypeImpl, with its per-path cache.
class CachedFileType extends FileType {
	constructor() {
		super(undefined, isMatch)
		this._cache = new Map()
		this.fileTypes = structuredClone(fileDefinitions)
	}
	async setup() {}
	get(filePath, searchFileType, checkFileExtension = true) {
		if (!filePath || !checkFileExtension || searchFileType !== undefined) {
			return super.get(filePath, searchFileType, checkFileExtension)
		}
		const cached = this._cache.get(filePath)
		if (cached !== undefined) return cached ?? undefined
		const result = super.get(filePath, searchFileType, checkFileExtension)
		this._cache.set(filePath, result ?? null)
		return result
	}
}

class DefinedPackType extends PackType {
	async setup() {
		this.packTypes = structuredClone(packDefinitions)
	}
}

class QuietConsole extends Console {
	log() {}
	error() {}
	warn() {}
	info() {}
}

// Every file under dir, relative to it, with its bytes.
function snapshot(dir) {
	const files = {}
	const walk = (sub) => {
		for (const entry of readdirSync(nodeJoin(dir, sub), { withFileTypes: true })) {
			const rel = sub ? `${sub}/${entry.name}` : entry.name
			if (entry.isDirectory()) walk(rel)
			else files[rel] = readFileSync(nodeJoin(dir, rel))
		}
	}
	walk("")
	return files
}

// Text as it is, anything that is not UTF-8 as base64.
function encode(bytes) {
	const text = bytes.toString("utf8")
	return Buffer.from(text, "utf8").equals(bytes) ? text : { base64: bytes.toString("base64") }
}

const variants = [
	["production", "production", false],
	["development", "development", false],
	["production-out", "production", true],
	["development-out", "development", true],
]

async function build(project, mode, separateOutput, settings) {
	const work = mkdtempSync(nodeJoin(tmpdir(), "leafcutter-corpus-"))
	const root = nodeJoin(work, "project")
	const out = nodeJoin(work, "out")
	cpSync(nodeJoin(corpusDir, project), root, { recursive: true })
	mkdirSync(out)
	const before = snapshot(root)
	const fs = new NodeFileSystem(root)
	// The Deno CLI's getProjectConfig.
	const config = await fs.readFile("dash-config.json").then(() => "./dash-config.json", () => "./config.json")
	const dash = new Dash(fs, separateOutput ? new NodeFileSystem(out) : undefined, {
		config,
		compilerConfig: settings.compilerConfig,
		mode,
		console: new QuietConsole(),
		verbose: false,
		packType: new DefinedPackType(undefined),
		fileType: new CachedFileType(),
		requestJsonData: async () => {
			throw new Error("requestJsonData is not part of the non-JavaScript corpus")
		},
	})
	await dash.setup()
	await dash.build()
	const after = snapshot(root)
	const written = {}
	for (const [path, bytes] of Object.entries(after)) {
		if (!before[path]?.equals(bytes)) written[path] = encode(bytes)
	}
	const removed = Object.keys(before).filter((path) => !(path in after))
	const output = Object.fromEntries(Object.entries(snapshot(out)).map(([path, bytes]) => [path, encode(bytes)]))
	rmSync(work, { recursive: true })
	return { written, removed, output }
}

for (const project of readdirSync(corpusDir, { withFileTypes: true }).filter((e) => e.isDirectory()).map((e) => e.name).sort()) {
	let settings = {}
	try {
		settings = JSON.parse(readFileSync(nodeJoin(corpusDir, project, "corpus.json"), "utf8"))
	} catch {}
	const expected = {}
	for (const [name, mode, separateOutput] of variants) {
		expected[name] = await build(project, mode, separateOutput, settings)
	}
	writeFileSync(nodeJoin(corpusDir, `${project}.expected.json`), JSON.stringify(expected, null, "\t") + "\n")
}
