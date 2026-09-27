// Writes project.json: what mc-project-core 0.5.0 answers for a set of project
// configs, using the file and pack definitions vendored in
// crates/leafcutter-ant/tests/data (the data TS Dash fetches from
// bridge-core/editor-packages).
//
// Dash builds the base path with pathe's dirname, which never returns an
// empty string, so no config here has one.
//
// Each vector is an object: the config's base path and data, then
// getAvailablePacks(), getAvailablePackPaths(), resolvePackPath results and,
// per file path, the file type id with picomatch as the matcher (as the Deno
// CLI sets it up), the file type id with a matcher that never matches (as the
// bridge. editor sets it up), and the pack type id. "throws" marks a call that
// threw.

import { createRequire } from "node:module"
import { isMatch } from "@bridge-editor/common-utils"
import { FileType, PackType, ProjectConfig } from "@bridge-editor/mc-project-core"
import { random as seeded, write } from "./record.mjs"

const require = createRequire(import.meta.url)
const fileDefinitions = require("../../crates/leafcutter-ant/tests/data/fileDefinitions.json")
const packDefinitions = require("../../crates/leafcutter-ant/tests/data/packDefinitions.json")

// mc-project-core logs a definition without "detect" before throwing.
console.log = () => {}

const { next: random, int, pick } = seeded(0x9ac7)

class Config extends ProjectConfig {
	constructor(basePath, data) {
		super(basePath)
		this.data = data
	}
}

const allPacks = { behaviorPack: "./BP", resourcePack: "./RP", skinPack: "./SP", worldTemplate: "./WT" }
const configs = [
	[".", { packs: { behaviorPack: "./BP", resourcePack: "./RP" } }],
	["projects/p", { packs: allPacks }],
	[".", { packs: { resourcePack: "RP", behaviorPack: "BP/" } }],
	[".", { packs: { customPack: "./CP", behaviorPack: "./BP" } }],
	[".", { packs: {} }],
	[".", {}],
	[".", { packs: { behaviorPack: null, resourcePack: "./RP" } }],
	[".", { packs: { behaviorPack: 5, resourcePack: ["./RP"] } }],
	[".", { packs: ["./BP"] }],
	[".", { packs: "BP" }],
	["/abs/p", { packs: allPacks }],
	["./", { packs: { behaviorPack: "../shared/BP", resourcePack: "/elsewhere/RP" } }],
	["a b", { packs: { behaviorPack: "./my BP", worldTemplate: "./WT" } }],
	[".", { packs: { behaviorPack: "", resourcePack: "." } }],
	[".", { packs: { "1": "./one", behaviorPack: "./BP", "0": "./zero" } }],
	[".", { packs: { constructor: null, behaviorPack: "./BP" } }],
]

// A path inside the directory each definition detects, for every pack it
// applies to, so most definitions are hit at least once.
function samplePaths(config) {
	const paths = new Set(["config.json", "a", ".bridge/extensions/x/manifest.json", "BP", "BP/", "BP/x", "BP/x."])
	for (const definition of fileDefinitions) {
		const detect = definition.detect ?? {}
		const scopes = [detect.scope ?? detect.matcher ?? []].flat()
		const extensions = detect.fileExtensions ?? [".json"]
		const packs = [detect.packType ?? []].flat()
		for (const scope of scopes) {
			const stem = String(scope).replace(/^!/, "").replace(/\*\*\/\*|\*/g, "sub/file").replace(/\/$/, "/x")
			const withExtension = /\.[a-z]+$/.test(stem) ? stem : stem + pick(extensions)
			for (const pack of packs.length ? packs : [null]) {
				const root = pack ? config.resolvePackPath(pack) : config.resolvePackPath()
				paths.add(`${root}/${withExtension}`)
				paths.add(`${root}/${withExtension}`.replace(/^\.\//, ""))
			}
		}
	}
	for (let i = 0; i < 60; i++) {
		paths.add(pick(["BP", "RP", "SP", "WT", "projects/p/BP", "CP", "one"]) + "/" + pick(["entities", "items", "blocks", "scripts", "functions", "texts", "models/entity", "components/item", "manifest.json", "sounds.json", "textures/blocks"]) + pick(["", "/x", "/sub/x"]) + pick([".json", ".js", ".ts", ".lang", ".mcfunction", ".png", "", "."]))
	}
	return [...paths]
}

function attempt(fn) {
	try {
		return fn()
	} catch {
		return "throws"
	}
}

const vectors = []
for (const [basePath, data] of configs) {
	const config = new Config(basePath, data)
	const globFileType = new FileType(config, isMatch)
	globFileType.fileTypes = structuredClone(fileDefinitions)
	const neverFileType = new FileType(config, () => false)
	neverFileType.fileTypes = structuredClone(fileDefinitions)
	const packType = new PackType(config)
	packType.packTypes = packDefinitions
	const resolve = []
	// Dash only asks an array or a string for its indices, so the methods
	// those inherit are out of reach and not probed.
	const probeInherited = data.packs !== null && typeof data.packs === "object" && !Array.isArray(data.packs)
	for (const packId of [undefined, "behaviorPack", "resourcePack", "worldTemplate", "customPack", "0", ...(probeInherited ? ["constructor"] : [])]) {
		for (const filePath of [undefined, "", "contents.json", "entities/", "./x", "../y"]) {
			resolve.push([packId ?? null, filePath ?? null, attempt(() => config.resolvePackPath(packId, filePath))])
		}
	}
	const files = samplePaths(config).map((path) => [
		path,
		attempt(() => globFileType.getId(path)),
		attempt(() => neverFileType.getId(path)),
		attempt(() => packType.getId(path)),
	])
	vectors.push({
		basePath,
		data,
		packs: config.getAvailablePacks(),
		packPaths: config.getAvailablePackPaths(),
		resolve,
		files,
	})
}

// A definition without "detect" throws once detection reaches it.
{
	const config = new Config(".", { packs: allPacks })
	const fileType = new FileType(config, isMatch)
	fileType.fileTypes = [{ id: "first", detect: { scope: "a/", fileExtensions: [".json"] } }, { id: "broken" }, { id: "late", add: "pre", detect: { scope: "late/" } }]
	const packType = new PackType(config)
	packType.packTypes = packDefinitions
	vectors.push({
		basePath: ".",
		data: { packs: allPacks },
		definitions: [{ id: "first", detect: { scope: "a/", fileExtensions: [".json"] } }, { id: "broken" }, { id: "late", add: "pre", detect: { scope: "late/" } }],
		packs: config.getAvailablePacks(),
		packPaths: config.getAvailablePackPaths(),
		resolve: [],
		files: ["a/x.json", "late/x.json", "b/x.json", "x", "a/x.txt"].map((path) => [path, attempt(() => fileType.getId(path)), "skip", attempt(() => packType.getId(path))]),
	})
}
write("project.json", vectors)
