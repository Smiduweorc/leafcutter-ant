// Writes globs.json and is-glob.json.
//
// Dash matches globs with common-utils' `isMatch`, which calls the picomatch
// copy vendored in common-utils as `picomatch(pattern)(path)` with no options,
// and decides whether a dependency query is a glob with is-glob 4.0.3.
//
// globs.json: [pattern, regex source or null, whether V8 compiled the source,
// [[path, isMatch result], ...]]. The source is the string picomatch hands to
// `new RegExp`, with every code unit outside printable ASCII written as
// `\uXXXX`, because picomatch can split a surrogate pair and JSON readers
// refuse a lone surrogate. null means picomatch threw before building one. The results
// come from the common-utils build Dash imports; the sources come from the
// same picomatch in common-utils' `src`, which is the file that build bundles.
//
// is-glob.json: [string, isGlob result].

import { createRequire } from "node:module"
import { isMatch } from "@bridge-editor/common-utils"
import picomatch from "./node_modules/@bridge-editor/common-utils/src/glob/picomatch.js"
import { random as seeded, write } from "./record.mjs"

const require = createRequire(import.meta.url)
const isGlob = require("is-glob")
const fileDefinitions = require("../../crates/leafcutter-ant/tests/data/fileDefinitions.json")

const { next: random, int, pick } = seeded(0x61085)

let lastSource
const toRegex = picomatch.toRegex
picomatch.toRegex = (source, options) => {
	lastSource = source
	return toRegex(source, options)
}
function compile(pattern) {
	lastSource = null
	try {
		const regex = picomatch.makeRe(pattern, {}, false, true)
		const escaped = lastSource.replace(/[^\x20-\x7e]/g, (c) => "\\u" + c.charCodeAt(0).toString(16).padStart(4, "0"))
		return [escaped, regex.source !== "$^" || lastSource === "$^"]
	} catch {
		return [null, false]
	}
}

// Real patterns: every file definition matcher, prefixed with pack roots the
// way mc-project-core prefixes them, and the float fix's two globs.
const patterns = new Set([
	"minecraft:entity/description/properties/*/value",
	"minecraft:entity/description/properties/*/range/*",
])
for (const definition of fileDefinitions) {
	const matchers = [definition.detect?.matcher ?? []].flat()
	const packs = [definition.detect?.packType ?? []].flat()
	for (let matcher of matchers) {
		if (matcher.startsWith("!")) matcher = matcher.slice(1)
		patterns.add(matcher)
		for (const root of packs.length ? ["BP", "RP", "projects/p/BP", "../x/RP"] : ["", "projects/p"]) {
			patterns.add(root ? `${root}/${matcher}` : matcher)
		}
	}
}

const atoms = [
	"a", "b", "BP", "entities", "x.json", ".json", "*", "*", "**", "?", "[abc]", "[a-z]", "[!a]", "[^b]", "[]]", "[",
	"]", "{a,b}", "{a,}", "{1..3}", "{a..c}", "{x}", "(a|b)", "!(a)", "@(a|b)", "+(a)", "*(b)", "?(c)", "\\*", "\\",
	"\\\\", "$", "^", ".", "..", "+", "|", ",", "\"q\"", "[[:alpha:]]", "[[:digit:]x]", "(?:a)", "(?=a)", "(?<n>a)",
	"\u00e9", "\u{1f41c}", "a b", "{", "}", "(", ")", ":", "@", "!",
]
function randomPattern() {
	let pattern = random() < 0.1 ? pick(["!", "./", "!!", "/"]) : ""
	for (let i = int(5); i >= 0; i--) {
		pattern += pick(atoms)
		if (random() < 0.6) pattern += "/"
	}
	return pattern
}
for (let i = 0; i < 1200; i++) patterns.add(randomPattern())
for (const extra of ["*", "**", "*.*", "**/*", "**/*.*", "**/.*", ".*", "*/*", "*.json", "**/*.json", "?", "!*", "!**/*.ts", "***", "**/**", "**/**/**", "a/**", "a/**/b", "**/b", "a**", "**a", "a/**b", "[a-", "a/[", "{a", "a}", "(a", "a)", "\\", "a\\", "\\/", "a\\.b", "\u0000", "\"a*\"", "@(a)", "!(a)*", "*(a|b)c", ".x/**", "constructor", "__proto__", "toString", "[[:constructor:]]", "[[:__proto__:]]", "[[:toString:]]x", "[[:alpha:]]", "a/[[:digit:]]*"]) {
	patterns.add(extra)
}

// Paths that should often match: each pattern instantiated at random.
function instantiate(pattern) {
	return pattern
		.replace(/^!+/, "")
		.replace(/\*\*/g, () => pick(["", "a", "a/b", "x/y/z", ".hidden"]))
		.replace(/\*/g, () => pick(["", "x", "file", "a.b", ".d"]))
		.replace(/\?/g, () => pick(["a", ".", "\u00e9"]))
		.replace(/\[!?\^?([^\]]*)\]/g, (_, inner) => inner.charAt(0) || "a")
		.replace(/\{([^},]*),?[^}]*\}/g, (_, first) => first)
		.replace(/[@!+]\(([^|)]*)[^)]*\)/g, (_, first) => first)
		.replace(/\\(.)/g, "$1")
}
const paths = [
	"a", "b", "a/b", "a/b/c", ".hidden", "a/.hidden", "BP/entities/x.json", "BP/entities/sub/x.json",
	"BP/entities/.x.json", "RP/models/entity/a.geo.json", "BP/scripts/main.js", "projects/p/BP/functions/tick.json",
	"minecraft:entity/description/properties/p/value", "minecraft:entity/description/properties/p/range/0",
	"minecraft:entity/description/properties/a/b/value", "x.json", "./a", "a/", "/a", "a\\b", "\u{1f41c}", "\u00e9",
	"a\nb", "..", "a/../b", "[abc]", "{a,b}", "a b",
]

const globs = []
for (const pattern of patterns) {
	const [source, compiled] = compile(pattern)
	const inputs = new Set([pattern, instantiate(pattern), instantiate(pattern), instantiate(pattern)])
	for (let i = 0; i < 8; i++) inputs.add(pick(paths))
	const results = []
	for (const input of inputs) {
		let result
		try {
			result = isMatch(input, pattern)
		} catch {
			result = "throws"
		}
		results.push([input, result])
	}
	globs.push([pattern, source, compiled, results])
}
write("globs.json", globs)

const globStrings = new Set([
	"", "a", "*", "a*", "!a", "a!", "?", "a?", "]?", ".?", "+?", ")?", "[a]", "[]", "[]a]", "[a\\]", "[\\a]", "{a}",
	"{}", "{a\\}", "(?:a)", "(?!a)", "(?=a)", "(?)", "(a|b)", "(|b)", "(a|)", "(a|b\\)", "\\*", "\\!", "\\{a}!",
	"\\(a)!", "\\[a]!", "@(a)", "!(a)", "+(a", "\\@(a)", "a\n@(b)", "BP/entities/x.json", "BP/**/*.json",
	"minecraft:entity", "a.b", "{a,b}", "a/{b", "a]", "(a)",
])
for (const pattern of patterns) globStrings.add(pattern)
for (let i = 0; i < 1500; i++) globStrings.add(pick(paths) + pick(atoms) + (random() < 0.5 ? pick(atoms) : ""))
write("is-glob.json", [...globStrings].map((s) => [s, isGlob(s)]))
