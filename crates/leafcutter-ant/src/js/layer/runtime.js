// The JavaScript half of the script engine, evaluated once per engine as a
// function of what Rust hands it: `host`, the Rust functions this file calls,
// and the path-browserify and pathe functions TS Dash's loader and generator
// modules use. It returns what Rust calls back into.
//
// Scripts see ECMAScript's own globals plus `console`, `File` and `Blob`
// (defined here); `host` stays private to this file.
//
// The module loader is js-runtime 0.4.5's `Runtime` class
// (dist/bridge-js-runtime.es.js), kept as JavaScript so that everything a
// script can observe about it (the order of its env parameters, the text of
// its errors, which env a cached module saw first) is what TS Dash gives.
// Copyright (c) 2022 bridge-team, MIT; see NOTICE.md.
(host, pathBrowserify, pathe) => {
	const { basename, dirname, extname } = pathBrowserify
	const join = pathBrowserify.join

	// Bytes as a string of one Latin-1 character each, the form Rust reads
	// them in.
	const latin1 = (bytes) => {
		let text = ""
		for (let i = 0; i < bytes.length; i += 8192) {
			text += String.fromCharCode.apply(null, bytes.subarray(i, i + 8192))
		}
		return text
	}
	const viewBytes = (view) => new Uint8Array(view.buffer, view.byteOffset, view.byteLength)

	// Blob and File as far as scripts and the loader use them: bytes, text,
	// a name. Strings are stored as UTF-8, with lone surrogates as U+FFFD, as
	// a browser stores them.
	let blobBytes
	const bytesOf = (part) => {
		if (part instanceof Blob) return blobBytes(part)
		if (part instanceof ArrayBuffer) return new Uint8Array(part)
		if (ArrayBuffer.isView(part)) return viewBytes(part)
		return host.encode(String(part).toWellFormed())
	}
	const concat = (parts) => {
		let size = 0
		for (const part of parts) size += part.byteLength
		const all = new Uint8Array(size)
		let offset = 0
		for (const part of parts) {
			all.set(part, offset)
			offset += part.byteLength
		}
		return all
	}
	class Blob {
		#bytes
		#type
		static {
			blobBytes = (blob) => blob.#bytes
		}
		constructor(parts = [], options = {}) {
			this.#bytes = concat(Array.from(parts, bytesOf))
			this.#type = options?.type === undefined ? "" : String(options.type).toLowerCase()
		}
		get size() {
			return this.#bytes.byteLength
		}
		get type() {
			return this.#type
		}
		async text() {
			return host.decode(latin1(this.#bytes))
		}
		async arrayBuffer() {
			return this.#bytes.slice().buffer
		}
		async bytes() {
			return this.#bytes.slice()
		}
		slice(start, end, type) {
			return new Blob([this.#bytes.slice(start, end)], { type })
		}
	}
	class File extends Blob {
		#name
		#lastModified
		constructor(parts, name, options = {}) {
			if (arguments.length < 2) throw new TypeError("Failed to construct 'File': 2 arguments required, but only " + arguments.length + " present.")
			super(parts, options)
			this.#name = String(name)
			this.#lastModified = options?.lastModified === undefined ? Date.now() : Number(options.lastModified)
		}
		get name() {
			return this.#name
		}
		get lastModified() {
			return this.#lastModified
		}
	}
	globalThis.Blob = Blob
	globalThis.File = File

	// One argument of a console call as the host's console gets it: strings
	// as they are, errors as `String(error)`, anything else as JSON where it
	// has a JSON form.
	const show = (value) => {
		if (typeof value === "string") return value
		if (value instanceof Error) return String(value)
		if (typeof value === "function") return `[Function: ${value.name || "(anonymous)"}]`
		if (value === null || typeof value !== "object") return String(value)
		try {
			return JSON.stringify(value) ?? String(value)
		} catch {
			return String(value)
		}
	}
	const line = (args) => args.map(show).join(" ")
	// TS Dash's Console: the four levels and the verbose timers.
	class Console {
		log(...args) {
			host.log("log", line(args))
		}
		info(...args) {
			host.log("info", line(args))
		}
		warn(...args) {
			host.log("warn", line(args))
		}
		error(...args) {
			host.log("error", line(args))
		}
		time(timerName) {
			host.time(String(timerName))
		}
		timeEnd(timerName) {
			host.timeEnd(String(timerName))
		}
	}
	const console = new Console()
	globalThis.console = console

	// TS Dash's FileSystem class with the Deno CLI's `readFile` and
	// `writeFile`, on a file system Rust holds. `readJson` and `writeJson`
	// are the base class's.
	class FileSystem {
		#id
		constructor(id) {
			this.#id = id
		}
		async readFile(path) {
			return new File([await host.readFile(this.#id, path)], basename(path))
		}
		// The Deno CLI writes a string as text and a Uint8Array as bytes; Deno
		// refuses anything else.
		async writeFile(path, content) {
			let bytes
			if (typeof content === "string") bytes = host.encode(content.toWellFormed())
			else if (content instanceof Uint8Array) bytes = content
			else throw new TypeError("The data to write must be a string or a Uint8Array")
			await host.writeFile(this.#id, path, latin1(bytes))
		}
		async unlink(path) {
			await host.unlink(this.#id, path)
		}
		async readdir(path) {
			return host.readdir(this.#id, path)
		}
		async mkdir(path) {
			await host.mkdir(this.#id, path)
		}
		async lastModified(path) {
			return host.lastModified(this.#id, path)
		}
		async allFiles(path) {
			const files = []
			const entries = await this.readdir(path)
			for (const { name, kind } of entries) {
				if (kind === "directory") {
					files.push(...(await this.allFiles(pathe.join(path, name))))
				} else if (kind === "file") {
					files.push(pathe.join(path, name))
				}
			}
			return files
		}
		async directoryHasAnyFile(path) {
			const entries = await this.readdir(path).catch(() => [])
			return entries.length > 0
		}
		async copyFile(from, to, outputFs = this) {
			const file = await this.readFile(from)
			await outputFs.writeFile(to, new Uint8Array(await file.arrayBuffer()))
		}
		async writeJson(path, content, beautify = true) {
			await this.writeFile(path, JSON.stringify(content, null, beautify ? "\t" : 0))
		}
		async readJson(path) {
			const file = await this.readFile(path)
			try {
				return await host.parseJson5(await file.text())
			} catch {
				throw new Error(`Invalid JSON: ${path}`)
			}
		}
		watchDirectory(path, onChange) {
			console.warn("Watching a directory for changes is not supported on this platform!")
		}
	}

	class Module {
		constructor(defaultExport, named) {
			this.__default__ = defaultExport
			for (const [key, value] of Object.entries(named)) {
				this[key] = value
			}
		}
	}

	class Runtime {
		constructor(modules, readFile) {
			this.evaluatedModules = new Map()
			this.baseModules = new Map()
			this.moduleLoaders = new Map()
			this.env = {}
			Object.defineProperty(this, "readFile", { value: readFile, writable: true, configurable: true })
			if (modules) {
				for (const [moduleName, module] of modules) {
					this.registerModule(moduleName, module)
				}
			}
		}
		async run(filePath, env = {}, file) {
			if (typeof file === "string") {
				file = new File([file], basename(filePath))
			}
			const module = await this.eval(filePath, env, file)
			return module
		}
		clearCache() {
			this.evaluatedModules.clear()
		}
		registerModule(moduleName, module) {
			this.baseModules.set(moduleName, module)
		}
		deleteModule(moduleName) {
			this.baseModules.delete(moduleName)
		}
		addModuleLoader(fileExtension, loader) {
			this.baseModules.set(fileExtension, loader)
		}
		async eval(filePath, env, file) {
			const evaluatedModule = this.evaluatedModules.get(filePath)
			if (evaluatedModule) return evaluatedModule
			const fileDirName = dirname(filePath)
			if (!file) file = await this.readFile(filePath).catch(() => void 0)
			if (!file) throw new Error(`File "${filePath}" not found`)
			const fileContent = await file.text()
			const transformedSource = await this.transformSource(filePath, fileContent)
			const module = {}
			try {
				await this.runSrc(
					transformedSource,
					Object.assign({}, env, {
						___module: module,
						___require: (moduleName) => this.require(moduleName, fileDirName, env),
					})
				)
			} catch (err) {
				throw new Error(`Error in ${filePath}: ${err}`)
			}
			this.evaluatedModules.set(filePath, module)
			return module
		}
		async transformSource(filePath, fileContent) {
			// `await loadedWasm`
			await null
			return host.transform(filePath, basename(filePath), fileContent)
		}
		async require(moduleName, baseDir, env) {
			const baseModule = this.baseModules.get(moduleName)
			if (baseModule) {
				if (typeof baseModule === "string") {
					const file = new File([baseModule], moduleName)
					return await this.eval(moduleName, env, file)
				} else if (typeof baseModule === "function") {
					return await baseModule()
				} else {
					return baseModule
				}
			}
			if (moduleName.startsWith("https://")) {
				const file = new File([await host.fetch(moduleName)], moduleName)
				return await this.eval(moduleName, env, file)
			}
			if (moduleName.startsWith(".")) moduleName = join(baseDir, moduleName)
			const extension = extname(moduleName)
			if (extension === ".json") {
				const fileContent = await this.readFile(moduleName)
					.then((file) => file.text())
					.catch(() => void 0)
				if (fileContent) {
					let json = {}
					try {
						json = host.parseJson5(fileContent)
					} catch {
						throw new Error(`File "${moduleName}" contains invalid JSON`)
					}
					return new Module(json, json)
				}
			}
			const customLoader = this.moduleLoaders.get(extension)
			if (customLoader) {
				const cachedModule = this.evaluatedModules.get(moduleName)
				if (cachedModule) return cachedModule
				const module = customLoader(moduleName)
				if (module instanceof Module) {
					this.evaluatedModules.set(moduleName, module)
					return module
				} else {
					return await this.eval(moduleName, env, module)
				}
			}
			const extensions = [".ts", ".js"]
			for (const ext of extensions) {
				const filePath = `${moduleName}${ext}`
				let fileContent = await this.readFile(filePath).catch(() => void 0)
				if (!fileContent) continue
				return await this.eval(filePath, env, fileContent)
			}
			throw new Error(`Module "${moduleName}" not found`)
		}
		async runSrc(src, env) {
			return new Function(...Object.keys(env), `return (async () => {\n${src}\n})()`)(...Object.values(env))
		}
	}

	// `[[NumberData]]` and its siblings, which JSON.stringify unwraps: the
	// prototype's own valueOf accepts only an object that has the slot.
	const hasSlot = (valueOf, value) => {
		try {
			valueOf.call(value)
			return true
		} catch {
			return false
		}
	}
	// SerializeJSONProperty's first steps: toJSON, then the boxed primitives.
	const prepare = (holder, key) => {
		let value = holder[key]
		if (value !== null && (typeof value === "object" || typeof value === "function" || typeof value === "bigint")) {
			const toJSON = value.toJSON
			if (typeof toJSON === "function") value = toJSON.call(value, key)
		}
		if (value !== null && typeof value === "object") {
			if (hasSlot(Number.prototype.valueOf, value)) value = Number(value)
			else if (hasSlot(String.prototype.valueOf, value)) value = String(value)
			else if (hasSlot(Boolean.prototype.valueOf, value)) value = Boolean.prototype.valueOf.call(value)
			else if (hasSlot(BigInt.prototype.valueOf, value)) value = BigInt.prototype.valueOf.call(value)
		}
		return value
	}
	const skipped = (value) => value === undefined || typeof value === "function" || typeof value === "symbol"
	const toLength = (length) => {
		const n = Math.trunc(Number(length))
		return n > 0 ? Math.min(n, Number.MAX_SAFE_INTEGER) : 0
	}
	// JSON.stringify(value), compact, with a stack of its own instead of
	// recursion, so that nesting of any depth is written as V8 writes it.
	// Returns undefined where JSON.stringify does, and throws its TypeErrors.
	const toJsonText = (value) => {
		const out = []
		const frames = []
		const open = new Set()
		// Writes a value that is not skipped; a container is opened.
		const write = (value) => {
			if (value === null) out.push("null")
			else if (value === true) out.push("true")
			else if (value === false) out.push("false")
			else if (typeof value === "string") out.push(JSON.stringify(value))
			else if (typeof value === "number") out.push(Number.isFinite(value) ? JSON.stringify(value) : "null")
			else if (typeof value === "bigint") throw new TypeError("Do not know how to serialize a BigInt")
			else {
				if (open.has(value)) throw new TypeError("Converting circular structure to JSON")
				open.add(value)
				if (Array.isArray(value)) {
					out.push("[")
					frames.push({ value, array: true, length: toLength(value.length), index: 0 })
				} else {
					out.push("{")
					frames.push({ value, array: false, keys: Object.keys(value), index: 0, wrote: false })
				}
			}
		}
		const root = prepare({ "": value }, "")
		if (skipped(root)) return undefined
		write(root)
		while (frames.length > 0) {
			const frame = frames[frames.length - 1]
			if (frame.array) {
				if (frame.index < frame.length) {
					const index = frame.index++
					if (index > 0) out.push(",")
					const item = prepare(frame.value, String(index))
					if (skipped(item)) out.push("null")
					else write(item)
					continue
				}
				out.push("]")
			} else {
				let opened = false
				while (frame.index < frame.keys.length) {
					const key = frame.keys[frame.index++]
					const item = prepare(frame.value, key)
					if (skipped(item)) continue
					if (frame.wrote) out.push(",")
					frame.wrote = true
					out.push(JSON.stringify(key), ":")
					const depth = frames.length
					write(item)
					if (frames.length > depth) {
						opened = true
						break
					}
				}
				if (opened) continue
				out.push("}")
			}
			open.delete(frame.value)
			frames.pop()
		}
		return out.join("")
	}

	// Objects Rust builds values into, with "__proto__" as an own property as
	// JSON.parse makes it.
	const defineData = (object, key, value) => {
		Object.defineProperty(object, key, { value, writable: true, enumerable: true, configurable: true })
	}

	// What an `await` does with a hook's result.
	const settle = (value) => Promise.resolve(value)

	// What TS Dash writes for a file's data (FileTransformer.transformFile):
	// data that `isWritableData` accepts is handed to the file system as it
	// is, which writes strings and byte arrays and fails on anything else
	// (a Blob, an ArrayBuffer), and other data is `JSON.stringify`d. Returns
	// [kind, text]: "text", "bytes" (Latin-1), "unwritable" or "none" (for
	// JSON.stringify giving undefined).
	const output = (data) => {
		if (typeof data === "string") return ["text", data.toWellFormed()]
		if (data instanceof Blob || data instanceof ArrayBuffer) return ["unwritable", ""]
		if (data?.buffer instanceof ArrayBuffer) {
			return ArrayBuffer.isView(data) ? ["bytes", latin1(viewBytes(data))] : ["unwritable", ""]
		}
		const text = toJsonText(data)
		return text === undefined ? ["none", ""] : ["text", text]
	}

	// `String(value)` for an error that reaches the console, where the value
	// may be anything a script threw.
	const describe = (value) => {
		try {
			return String(value)
		} catch {
			return Object.prototype.toString.call(value)
		}
	}

	return {
		Runtime,
		FileSystem,
		File,
		basename,
		Module,
		console,
		pathe,
		toJsonText,
		defineData,
		settle,
		output,
		describe,
		wellFormed: (s) => s.toWellFormed(),
	}
}
