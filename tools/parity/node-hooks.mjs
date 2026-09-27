// Two packages in TS Dash's tree cannot be imported from Node as TS Dash
// imports them in a browser bundle: @swc/wasm-web has only a "module" field,
// which Node's resolver ignores, and path-browserify is CommonJS, so the named
// imports js-runtime uses fail. These hooks point the first at the file its
// "module" field names and wrap the second in an ES module. Loaded with
// `node --import ./node-hooks.mjs`.

import { registerHooks } from "node:module"

const pathShim = new URL("./path-browserify.mjs", import.meta.url).href

registerHooks({
	resolve(specifier, context, next) {
		if (specifier === "@swc/wasm-web") return next("@swc/wasm-web/wasm-web.js", context)
		if (specifier === "path-browserify" && context.parentURL !== pathShim) {
			return { url: pathShim, shortCircuit: true }
		}
		return next(specifier, context)
	},
})
