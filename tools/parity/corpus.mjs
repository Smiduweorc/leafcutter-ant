// Builds every project under crates/leafcutter-ant/tests/corpus with TS Dash
// 0.13.0 and writes what each build changed to <project>.expected.json, which
// the Rust test crates/leafcutter-ant/tests/corpus.rs compares against.
//
// Each project is built four times, the ways the Deno CLI can build it:
// `production` and `development` write into the project, and the `-out`
// variants give Dash a separate output file system, as `--out` does. Every
// build starts from a fresh copy of the project in a temporary directory.
//
// ts-dash.mjs plays the Deno CLI's part.
//
// Usage: npm ci && npm run vectors   (from tools/parity)

import { cpSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs"
import { tmpdir } from "node:os"
import { join as nodeJoin } from "node:path"
import { fileURLToPath } from "node:url"
import { Dash } from "@bridge-editor/dash-compiler"
import { CachedFileType, DefinedPackType, NodeFileSystem, QuietConsole } from "./ts-dash.mjs"

const corpusDir = fileURLToPath(new URL("../../crates/leafcutter-ant/tests/corpus/", import.meta.url))

// floatPropertyTruncationFix and formatVersionCorrection log through the
// global console.
console.log = () => {}
console.error = () => {}

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
