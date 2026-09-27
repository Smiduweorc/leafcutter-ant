// Writes typescript.json: what @swc/wasm-web 1.6.5, the TypeScript compiler
// TS Dash bundles, makes of each file in tools/parity/typescript with the
// options Dash's typeScript plugin passes, with and without inline source
// maps. Each vector is [file name, source, inline source maps, the code or
// null when the compiler threw].

import { readdirSync, readFileSync } from "node:fs"
import { createRequire } from "node:module"
import init, { transformSync } from "@swc/wasm-web/wasm-web.js"
import { write } from "./record.mjs"

const require = createRequire(import.meta.url)
await init(readFileSync(require.resolve("@swc/wasm-web/wasm-web_bg.wasm")))

const dir = new URL("./typescript/", import.meta.url)
const vectors = []
for (const name of readdirSync(dir).sort()) {
	const source = readFileSync(new URL(name, dir), "utf8")
	for (const inline of [false, true]) {
		let code = null
		try {
			code = transformSync(source, {
				filename: name,
				sourceMaps: inline ? "inline" : undefined,
				jsc: {
					parser: { syntax: "typescript" },
					preserveAllComments: false,
					target: "es2020",
					transform: { useDefineForClassFields: false },
				},
			}).code
		} catch {}
		vectors.push([name, source, inline, code])
	}
}
write("typescript.json", vectors)
