// The host side of TS Dash as the Deno CLI sets it up, for the scripts that
// run TS Dash itself: its file system, file and pack types, and a console
// that keeps quiet.
//
// Two changes fix what the Deno CLI leaves to timing: directories are listed
// sorted by the bytes of their UTF-8 names, and `allFiles` finishes in pack
// order. leafcutter-ant does the same; see the README's differences table.

import { mkdirSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs"
import { createRequire } from "node:module"
import { dirname, isAbsolute, join as nodeJoin } from "node:path"
import { isMatch } from "@bridge-editor/common-utils"
import { Console, FileSystem, initRuntimes } from "@bridge-editor/dash-compiler"
import { FileType, PackType } from "@bridge-editor/mc-project-core"
import { join } from "pathe"

const require = createRequire(import.meta.url)
const fileDefinitions = require("../../crates/leafcutter-ant/tests/data/fileDefinitions.json")
const packDefinitions = require("../../crates/leafcutter-ant/tests/data/packDefinitions.json")

initRuntimes(readFileSync(require.resolve("@swc/wasm-web/wasm-web_bg.wasm")))

const byUtf8 = (a, b) => Buffer.compare(Buffer.from(a.name), Buffer.from(b.name))

// The Deno CLI's DenoFileSystem, on Node's synchronous calls.
export class NodeFileSystem extends FileSystem {
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
export class CachedFileType extends FileType {
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

export class DefinedPackType extends PackType {
	async setup() {
		this.packTypes = structuredClone(packDefinitions)
	}
}

export class QuietConsole extends Console {
	log() {}
	error() {}
	warn() {}
	info() {}
}
