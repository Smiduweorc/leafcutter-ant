// Writes the golden vectors under crates/leafcutter-ant/tests/vectors/ from
// the JavaScript that TS Dash 0.13.0 runs: V8's JSON.stringify and Number
// formatting, and json5 at the version Dash bundles (2.2.1). The output is
// deterministic (a fixed seed), so running this again must leave `git status`
// clean.
//
// Usage: npm ci && npm run vectors   (from tools/parity)

import JSON5 from "json5"
import { random as seeded, write } from "./record.mjs"

// json5 warns on stderr about U+2028 and U+2029 inside strings; the vectors
// cover that case on purpose.
console.warn = () => {}

const { next: random, int, pick, shuffled } = seeded(0x1eafc07)

function bitsOf(x) {
	const view = new DataView(new ArrayBuffer(8))
	view.setFloat64(0, x)
	return view.getBigUint64(0).toString(16).padStart(16, "0")
}

function fromBits(hex) {
	const view = new DataView(new ArrayBuffer(8))
	view.setBigUint64(0, BigInt("0x" + hex))
	return view.getFloat64(0)
}

// numbers.json: [bits of an f64, JSON.stringify of it]. Hand-picked values
// where the ECMAScript format changes notation, then random bit patterns
// (every exponent equally likely) and random short decimals.
const numbers = [
	0, -0, 1, -1, 0.1, 0.2, 0.30000000000000004, 1.5, 100, 123456789, 2 ** 53, 2 ** 53 + 2, -(2 ** 53),
	1e20, 1e21, 1.5e21, 123456789012345680000, 1e-6, 1e-7, 1.5e-7, 0.0000012345,
	5e-324, 2.2250738585072014e-308, 1.7976931348623157e308, Number.EPSILON, Math.PI, -Math.E,
	Infinity, -Infinity, NaN, 1 / 3, 2 / 3, 4.35, 0.5, 1e300, 1e-300, 9.999999999999999e20, 1e100,
].map(bitsOf)
for (let i = 0; i < 4000; i++) {
	numbers.push(int(2 ** 32).toString(16).padStart(8, "0") + int(2 ** 32).toString(16).padStart(8, "0"))
}
for (let i = 0; i < 2000; i++) {
	const mantissa = int(10 ** (1 + int(15)))
	numbers.push(bitsOf(Number(`${random() < 0.5 ? "-" : ""}${mantissa}e${int(40) - 20}`)))
}
write(
	"numbers.json",
	numbers.map((bits) => [bits, JSON.stringify(fromBits(bits))]),
)

// Random values shared by the stringify and json5 vectors.
const keyPool = [
	"a", "b", "z", "format_version", "minecraft:entity", "0", "1", "2", "10", "9", "01", "-1", "1.0", "+1", " 1",
	"4294967294", "4294967295", "4294967296", "123", "", "__proto__", "constructor", "toString", "\u00e9", "\u{1f41c}",
	"a\"b", "it's", "\\", "$x", "_y", "a1", "\u00e9t\u00e9", "\u1680", "x\u200dy",
]
const stringPool = [
	"", "plain", "quote\"", "it's", "back\\slash", "slash/", "\b\f\n\r\t\v", "\u0000\u0001\u001f\u007f",
	"\u2028\u2029", "\u00e9\u00df", "\u{1f41c} ant", "tab\tin", "q.variant == 1 ? 1.0 : 0", "</script>",
]
function randomNumber() {
	switch (int(5)) {
		case 0: return int(1000) - 500
		case 1: return (random() - 0.5) * 10 ** (int(50) - 25)
		case 2: return pick([0, -0, 1e21, 1e-7, 5e-324, 1.7976931348623157e308, 0.1, 2 ** 53, 2 ** 64])
		default: return fromBits(pick(numbers))
	}
}
function randomValue(depth) {
	switch (depth > 4 ? int(4) : int(6)) {
		case 0: return null
		case 1: return random() < 0.5
		case 2: return randomNumber()
		case 3: return pick(stringPool)
		case 4: return Array.from({ length: int(5) }, () => randomValue(depth + 1))
		default: {
			const entries = []
			const keys = new Set()
			for (let i = int(7); i > 0; i--) keys.add(pick(keyPool))
			for (const key of keys) entries.push([key, randomValue(depth + 1)])
			return { entries }
		}
	}
}

// stringify.json: [JSON source, compact output, tab-indented output]. The Rust
// test reads the source in text order and must reproduce V8's key order
// (array indices first, ascending), escaping and number format.
function jsonSource(value) {
	if (Array.isArray(value)) return "[" + value.map(jsonSource).join(",") + "]"
	if (value !== null && typeof value === "object") {
		const entries = shuffled(value.entries)
		return "{" + entries.map(([k, v]) => JSON.stringify(k) + ":" + jsonSource(v)).join(",") + "}"
	}
	// JSON has no spelling for these; the Rust unit tests cover them by hand.
	if (typeof value === "number" && !Number.isFinite(value)) return "0"
	return JSON.stringify(value)
}
const documents = [
	"{}", "[]", "[[]]", "{\"a\":{}}", "{\"a\":[]}", "0", "-0", "\"\"", "null", "true",
	"{\"b\":1,\"a\":2,\"1\":3,\"0\":4}", "[1,[2,[3,{\"x\":[]}]]]",
]
for (let i = 0; i < 1500; i++) documents.push(jsonSource(randomValue(0)))
write(
	"stringify.json",
	documents.map((source) => {
		const value = JSON.parse(source)
		return [source, JSON.stringify(value), JSON.stringify(value, null, "\t")]
	}),
)

// json5.json: [json5 source, "ok", compact JSON.stringify of the result] or
// [json5 source, "error", error message]. Random documents are written with
// every spelling json5 accepts, then some are damaged by one character so the
// error messages and their line:column positions are covered too.
const space = [" ", " ", "\t", "\n", "\r\n", "\r", "\v", "\f", "\u00a0", "\ufeff", "\u2028", "\u2029", "\u1680", "\u3000"]
function gap() {
	let s = ""
	for (let i = int(3); i > 0; i--) {
		const roll = int(12)
		if (roll === 0) s += "// note" + pick(["\n", "\r", "\u2028"])
		else if (roll === 1) s += pick(["/* note */", "/**/", "/* a * b **/", "/*\n*/"])
		else s += pick(space)
	}
	return s
}
function json5Number(n) {
	if (Number.isNaN(n)) return pick(["NaN", "+NaN", "-NaN"])
	if (n === Infinity) return pick(["Infinity", "+Infinity"])
	if (n === -Infinity) return "-Infinity"
	if (Object.is(n, -0)) return pick(["-0", "-0.0", "-.0", "-0e5", "-0x0"])
	const sign = n < 0 ? "-" : pick(["", "", "+"])
	const abs = Math.abs(n)
	if (Number.isInteger(abs) && abs <= 2 ** 64 && random() < 0.3) return sign + pick(["0x", "0X"]) + abs.toString(16)
	let text = String(abs)
	if (random() < 0.2 && text.startsWith("0.")) text = text.slice(1)
	if (random() < 0.2 && /^\d+$/.test(text)) text += "."
	if (random() < 0.2) text = text.replace("e", "E")
	return sign + text
}
function json5String(s) {
	const quote = random() < 0.5 ? "\"" : "'"
	let out = quote
	for (const ch of s) {
		const roll = int(8)
		if (ch === quote || ch === "\\" || ch === "\n" || ch === "\r") out += "\\" + (ch === "\n" ? "n" : ch === "\r" ? "r" : ch)
		else if (roll === 0 && ch.codePointAt(0) < 0x100) out += "\\x" + ch.codePointAt(0).toString(16).padStart(2, "0")
		else if (roll === 1) {
			for (let i = 0; i < ch.length; i++) out += "\\u" + ch.charCodeAt(i).toString(16).padStart(4, "0")
		} else if (roll === 2 && /[a-z]/.test(ch) && !"bfnrtvxu".includes(ch)) out += "\\" + ch
		else if (roll === 3 && ch === "\t") out += "\\t"
		else out += ch
	}
	if (random() < 0.1) out += "\\\n"
	return out + quote
}
function json5Key(key) {
	if (/^[A-Za-z_$\u00e9][\w$\u00e9\u200d]*$/.test(key) && random() < 0.7) return key
	return json5String(key)
}
function json5Source(value) {
	if (Array.isArray(value)) {
		const items = value.map((v) => gap() + json5Source(v) + gap())
		return "[" + items.join(",") + (items.length && random() < 0.3 ? "," : "") + gap() + "]"
	}
	if (value !== null && typeof value === "object") {
		const items = value.entries.map(([k, v]) => gap() + json5Key(k) + gap() + ":" + gap() + json5Source(v) + gap())
		return "{" + items.join(",") + (items.length && random() < 0.3 ? "," : "") + gap() + "}"
	}
	if (typeof value === "number") return json5Number(value)
	if (typeof value === "string") return json5String(value)
	return String(value)
}
function damage(source) {
	const chars = [...source]
	const at = int(chars.length + 1)
	switch (int(3)) {
		case 0: chars.splice(at, 1); break
		case 1: chars.splice(at, 0, pick(["x", "}", "]", ",", ":", "\"", "'", "/", "*", "\\", "0", "-", "\n", "\u{1f41c}", "\u0001", "\u001c", "\u2028"])); break
		default: chars.splice(at, 1, pick(["{", "[", "e", "+", ".", "\t", "\u0000"]))
	}
	return chars.join("")
}
const json5Sources = [
	"", " ", "//", "/*", "/* x", "/", "{", "[", "}", "]", "{}", "[]", "{,}", "[,]", "[1,]", "[1,,]", "{a:1,}",
	"null", "nul", "nullx", "true", "True", "false", "Infinity", "-Infinity", "+Infinity", "Infinit", "NaN", "-NaN",
	"0", "-0", "+0", "00", "01", "0.", ".5", "-.5", "+.5", "5.", "5.e3", "5e", "5e+", "5e-3", ".e3", ".", "-", "+", "- 1",
	"0x", "0x1F", "0X1f", "-0x10", "+0xFF", "0x1g", "1e400", "-1e400", "1e-400",
	"0x" + "f".repeat(13), "0x" + "f".repeat(14), "0x" + "f".repeat(20), "0x1fffffffffffff", "0x20000000000001",
	"0x20000000000003", "0x" + "1".repeat(40), "0x" + "8".repeat(64), "0x10000000000000800", "0x10000000000000801",
	"0x" + "0".repeat(300) + "1", "123456789012345678901234567890", "0.1000000000000000055511151231257827",
	"'single'", "\"double\"", "'it\\'s'", "\"a\\\"b\"", "'unterminated", "\"line\nbreak\"", "\"cr\rbreak\"",
	"\"sep\u2028\u2029\"", "\"\\b\\f\\n\\r\\t\\v\\0\\/\\a\\'\\\"\"", "\"\\01\"", "\"\\1\"", "\"\\9\"", "\"\\x4\"",
	"\"\\x41\\xe9\"", "\"\\u00e9\"", "\"\\u00E9\"", "\"\\u12\"", "\"\\uD83D\\uDC1C\"", "\"a\\\nb\"", "\"a\\\r\nb\"",
	"\"a\\\rb\"", "\"a\\\u2028b\"", "\"\\", "\"\u{1f41c}\"",
	"{a:1}", "{$:1}", "{_:1}", "{a1:1}", "{1a:1}", "{\u00e9:1}", "{\u{1f41c}:1}", "{a\u200db:1}", "{\\u0061:1}",
	"{\\u0031:1}", "{a\\u0062:1}", "{a\\u002d:1}", "{\\x61:1}", "{a\\x62:1}", "{a b:1}", "{a:1 b:2}", "{\"a\" 1}",
	"{a:}", "{:1}", "{a:1,,b:2}", "{\"a\":1,\"a\":2}", "{\"b\":1,\"a\":2,\"b\":3}",
	"{\"__proto__\":{\"x\":1},\"y\":2}", "{\"__proto__\":1}", "{__proto__:[1],y:2}", "{\"__proto__\":null}",
	"{\"constructor\":1,\"toString\":2}", "{\"2\":0,\"1\":0,\"b\":0,\"a\":0,\"0\":0}",
	"[1 2]", "[1,2]x", "1 2", "{} {}", "[]//c", "[]/*c*/", "[]/*c", "[] /", "\ufeff{}", "{}\ufeff",
	"\u00a0[\u2028]\u3000", "[\u180e]", "[\u0085]", "\u001f", "[\u001b]", "{\u000e:1}", "[1\u007f]", "[1\u00a0,\u00a02]", "\u0001", "[\u0000]", "\"\u0000\"",
	"[\"a\"\u{1f41c}]", "{a:1}\n\n  x", "\n\n\t{\r\n  a: 'b',\n  'c': \"d\" // trailing\n}\n",
	"[" .repeat(50) + "]".repeat(50), "[" .repeat(50) + "]".repeat(49), "{a:" .repeat(30) + "1" + "}".repeat(30),
]
for (let i = 0; i < 1500; i++) json5Sources.push(gap() + json5Source(randomValue(0)) + gap())
for (let i = 0; i < 1500; i++) json5Sources.push(damage(gap() + json5Source(randomValue(0)) + gap()))
// A lone surrogate made by an escape survives in a JS string and is written as
// "\udXXX"; Rust strings cannot hold one. That case is a ledger entry with its
// own test, so it is kept out of the generated vectors.
const loneSurrogate = /\\ud[89a-f][0-9a-f]{2}/i
write(
	"json5.json",
	json5Sources.flatMap((source) => {
		try {
			const output = JSON.stringify(JSON5.parse(source))
			return loneSurrogate.test(output ?? "") ? [] : [[source, "ok", output]]
		} catch (error) {
			if (!(error instanceof SyntaxError)) throw error
			return [[source, "error", error.message]]
		}
	}),
)
