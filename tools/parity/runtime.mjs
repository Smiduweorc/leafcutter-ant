// Writes runtime.json: what js-runtime 0.4.5, the module loader TS Dash runs
// user scripts and extension plugins in, turns each file in
// tools/parity/runtime into before it runs it (`Runtime.transformSource`:
// swc's transform and minifier from @swc/wasm-web 1.6.5, then the
// import/export rewrite). The two modules Dash itself hands the loader as
// source text, `@bridge/generate` and `@bridge-interal/collection`, are
// transformed too, under those names. Each vector is [module path, source,
// the code or null when the loader threw].

import { readdirSync, readFileSync } from "node:fs"
import { Runtime } from "@bridge-editor/js-runtime"
import { write } from "./record.mjs"
import { dashModuleSources } from "./ts-dash.mjs"

class Loader extends Runtime {
	async readFile() {
		throw new Error("the vectors read no files")
	}
}
const loader = new Loader()

const sources = []
const dir = new URL("./runtime/", import.meta.url)
for (const name of readdirSync(dir).sort()) sources.push([name, readFileSync(new URL(name, dir), "utf8")])
for (const [name, source] of Object.entries(dashModuleSources)) sources.push([name, source])
sources.push(["https://example.com/lib/mod.js", "export const fetched = true\nexport default 'remote'\n"])
sources.push(["folder.with.dots/plain", "export default 1\n"])

const vectors = []
for (const [path, source] of sources) {
	let code = null
	try {
		code = await loader.transformSource(path, source)
	} catch {}
	vectors.push([path, source, code])
}
write("runtime.json", vectors)
