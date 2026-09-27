// Writes plugins.json: the answers of TS Dash's own built-in plugins, called
// hook by hook, for plugins whose behaviour the non-JavaScript corpus cannot
// reach because no built-in there reads the files they look at. Each plugin
// is created by a real TS Dash setup and taken out of its hook table.
//
// entityIdentifierAlias: [path, content, ignore(path), registerAliases(path,
// content)].
// floatPropertyTruncationFix: [path, content, finalizeBuild(path, content),
// the lines it logged].
// "<undefined>" stands for undefined, "<fileContent>" for the content passed
// in coming back.

import { mkdtempSync, rmSync, writeFileSync } from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { Dash } from "@bridge-editor/dash-compiler"
import { write } from "./record.mjs"
import { CachedFileType, DefinedPackType, NodeFileSystem, QuietConsole } from "./ts-dash.mjs"

let logged = []
console.log = (...args) => logged.push(args.join(" "))

async function plugin(id) {
	const dir = mkdtempSync(join(tmpdir(), "leafcutter-plugins-"))
	writeFileSync(join(dir, "config.json"), JSON.stringify({ packs: { behaviorPack: "./BP", resourcePack: "./RP" }, compiler: { plugins: [id] } }))
	const dash = new Dash(new NodeFileSystem(dir), undefined, {
		config: "./config.json",
		mode: "development",
		console: new QuietConsole(),
		packType: new DefinedPackType(undefined),
		fileType: new CachedFileType(),
		requestJsonData: async () => null,
	})
	await dash.setup()
	rmSync(dir, { recursive: true })
	for (const plugins of dash.plugins.getImplementedHooks().values()) return plugins[0].plugin
}

const encode = (result, content) => (result === undefined ? "<undefined>" : result === content ? "<fileContent>" : result)

const entityPaths = ["BP/entities/a.json", "BP/entities/sub/b.json", "BP/items/i.json", "RP/entity/c.json", "BP/entities/x.txt", "config.json", "BP/entities/player.json"]
const identifiers = ["ns:a", "", 0, 5, -0, true, false, null, { a: 1 }, [1], [], " ", "é:\u{1f41c}"]
const entityContents = [
	...identifiers.map((identifier) => ({ "minecraft:entity": { description: { identifier } } })),
	{ "minecraft:entity": { description: {} } },
	{ "minecraft:entity": { description: null } },
	{ "minecraft:entity": { description: "text" } },
	{ "minecraft:entity": "text" },
	{ "minecraft:entity": [] },
	{ entity: { description: { identifier: "ns:x" } } },
	[{ "minecraft:entity": { description: { identifier: "ns:x" } } }],
	"text",
	5,
	null,
	true,
	{},
]
const alias = await plugin("entityIdentifierAlias")
const aliasVectors = []
for (const path of entityPaths) {
	for (const content of entityContents) {
		aliasVectors.push([path, content, alias.ignore(path), encode(alias.registerAliases(path, content), content)])
	}
}

// Entity documents for the float fix: properties of every type, floats that
// print with and without a dot, numbers the marker regex refuses, and the
// order of keys the replacer's never-popped path depends on.
const property = (type, value, range) => ({ type, ...(value === undefined ? {} : { default: value, value }), ...(range ? { range } : {}) })
const floatDocuments = [
	{ format_version: "1.16.0", "minecraft:entity": { description: { identifier: "p:p", properties: { "p:f": property("float", 1, [0, 10]) } } } },
	{ "minecraft:entity": { description: { properties: { "p:f": { type: "float", value: 1 }, "p:g": { type: "float", value: 2 } } } } },
	{ "minecraft:entity": { description: { properties: { "p:f": { value: 1.5, type: "float" } } } } },
	{ "minecraft:entity": { description: { properties: { "p:i": { type: "int", value: 1, range: [0, 5] } } } } },
	{ "minecraft:entity": { description: { properties: { "p:f": { type: "float", range: [0, 1.25, -3, 1e21, 1e-7] } } } } },
	{ "minecraft:entity": { description: { properties: { "p:f": { type: "float", value: -2 } } } } },
	{ "minecraft:entity": { description: { properties: { "p:f": { type: "float", value: 0 } } } } },
	{ "minecraft:entity": { description: { properties: { "p:f": { type: "float", value: 1e21 } } } } },
	{ "minecraft:entity": { description: { properties: { "p:f": { type: "float", value: 5e-7 } } } } },
	{ "minecraft:entity": { description: { properties: { "p:f": { type: "float", value: "1" } } } } },
	{ "minecraft:entity": { description: { identifier: "p:p" }, components: { "minecraft:health": { value: 20 } } } },
	{ "minecraft:entity": { components: { a: { value: 1 } }, description: { properties: { "p:f": { type: "float", value: 3 } } } } },
	{ "minecraft:entity": { description: { properties: { "p:f": { type: "float", value: 1, range: [{ x: 1 }, 2] } } } } },
	{ "minecraft:entity": { description: { properties: [{ type: "float", value: 1 }] } } },
	{ "minecraft:entity": { description: { properties: { "p:f": { type: "float", value: null } } } } },
	{ "minecraft:entity": { description: { properties: { "p/f": { type: "float", value: 7 } } } } },
	{ note: "$___dash___floatPropertyTruncationFix___THIS IS AUTO GENERATED AND I HATE IT___1.5" },
	{ note: "$___dash___floatPropertyTruncationFix___THIS IS AUTO GENERATED AND I HATE IT___x" },
	{ "$___dash___floatPropertyTruncationFix___THIS IS AUTO GENERATED AND I HATE IT___-2": 1 },
	[1, [2.5, { "minecraft:entity": 3 }]],
	{ "1": 1.5, b: { "0": 2 }, a: [true, null, "s"] },
	{},
	[],
	"already a string",
	7,
	null,
]
const floatPaths = ["BP/entities/player.json", "BP/entities/sub/player.json", "BP/entities/other.json", "BP/entities/myplayer.json", "BP/items/player.json", "BP/entities/player.json.txt"]
const float = await plugin("floatPropertyTruncationFix")
const floatVectors = []
for (const path of floatPaths) {
	for (const content of floatDocuments) {
		logged = []
		const result = float.finalizeBuild(path, content)
		floatVectors.push([path, content, encode(result, content), logged])
	}
}

write("plugins.json", [
	["entityIdentifierAlias", aliasVectors],
	["floatPropertyTruncationFix", floatVectors],
])
