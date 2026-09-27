// Writes paths.json: what the two pathe versions in TS Dash's dependency tree
// return. Dash itself imports pathe 2.0.2; mc-project-core, which resolves
// pack paths and file types, imports pathe 1.1.2, and the two disagree on
// `join` and `extname`.
//
// Each vector is [version, function, arguments, result].

import * as pathe2 from "pathe"
import * as pathe1 from "pathe-1"
import { random as seeded, write } from "./record.mjs"

// pathe resolves relative paths against process.cwd() when there is a
// `process`, and against "/" in a browser, where Dash runs inside the editor.
// leafcutter-ant uses "/" everywhere.
process.cwd = () => "/"

const { next: random, int, pick } = seeded(0x9a7e)

const segments = [
	"", "a", "b", "BP", "RP", "entities", "x.json", "x.y.json", ".hidden", "file.", "..", ".", "...", "a..b",
	"C:", "c:", "D:\\", "\\", "\\\\", "//", "/", "a b", "\u00e9t\u00e9", "\u{1f41c}.ts", "d.ts", "x.d.ts", "tick.json",
	"line\nbreak.txt", "\u2028.x", "*", "~", ".bridge", "builds", "dist", "Bridge BP",
]
const separators = ["/", "/", "/", "\\", "//", "/./", "/../"]

function randomPath() {
	let path = random() < 0.2 ? pick(["/", "./", "../", "C:/", "c:\\", "\\\\", "//"]) : ""
	for (let i = int(5); i >= 0; i--) {
		path += pick(segments)
		if (i > 0) path += pick(separators)
	}
	if (random() < 0.15) path += pick(["/", "\\", "/."])
	return path
}

const fixed = [
	"", ".", "..", "/", "//", "///", "\\", "./", "../", "a", "a/", "a//b", "/a/b/", "./BP", "./BP/", "BP/../RP",
	"../../x", "/../x", "C:", "C:/", "c:/x", "C:\\x\\y", "\\\\server\\share", "//server/share", "//./x", "a/.",
	"a/..", "x.json", ".json", "x.", "x..", "a.b/c", "a/b.c/", "/x.d.ts", "file.tar.gz", "line\n.txt",
	"\u2028.txt", "\u00e9.json",
]

const vectors = []
function record(version, lib, fn, args) {
	vectors.push([version, fn, args, lib[fn](...args)])
}
const inputs = [...fixed]
for (let i = 0; i < 600; i++) inputs.push(randomPath())
for (const p of inputs) {
	for (const fn of ["normalize", "dirname", "basename", "extname", "isAbsolute", "resolve"]) {
		record("2.0.2", pathe2, fn, [p])
	}
	record("1.1.2", pathe1, "extname", [p])
}
for (let i = 0; i < 400; i++) {
	const a = pick(inputs)
	const b = pick(inputs)
	record("2.0.2", pathe2, "basename", [a, pick([".json", ".ts", "x", "", b])])
	record("2.0.2", pathe2, "relative", [a, b])
	record("2.0.2", pathe2, "resolve", [a, b])
	const parts = Array.from({ length: int(4) }, () => pick(inputs))
	record("2.0.2", pathe2, "join", parts)
	record("1.1.2", pathe1, "join", parts)
}
for (const [a, b] of [["./BP", "BP/entities/x.json"], [".", "BP/x"], ["/a", "/a"], ["a", "a/b/c"], ["a/b", "a"], ["C:/a", "D:/b"], ["", ""]]) {
	record("2.0.2", pathe2, "relative", [a, b])
}
record("2.0.2", pathe2, "join", [])
record("1.1.2", pathe1, "join", [])
write("paths.json", vectors)
