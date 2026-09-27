// What every vector script shares: a seeded random source, so a second run
// writes the same bytes, and the writer for the vector files.

import { writeFileSync } from "node:fs"
import { fileURLToPath } from "node:url"

const outDir = fileURLToPath(new URL("../../crates/leafcutter-ant/tests/vectors/", import.meta.url))

// mulberry32: small, seedable, and identical on every Node version.
export function random(seed) {
	let a = seed >>> 0
	const next = () => {
		a = (a + 0x6d2b79f5) >>> 0
		let t = a
		t = Math.imul(t ^ (t >>> 15), t | 1)
		t ^= t + Math.imul(t ^ (t >>> 7), t | 61)
		return ((t ^ (t >>> 14)) >>> 0) / 4294967296
	}
	const int = (n) => Math.floor(next() * n)
	const pick = (list) => list[int(list.length)]
	const shuffled = (list) => {
		const copy = [...list]
		for (let i = copy.length - 1; i > 0; i--) {
			const j = int(i + 1)
			;[copy[i], copy[j]] = [copy[j], copy[i]]
		}
		return copy
	}
	return { next, int, pick, shuffled }
}

export function write(name, vectors) {
	// One vector per line keeps diffs of a regenerated file readable.
	writeFileSync(outDir + name, "[\n" + vectors.map((v) => JSON.stringify(v)).join(",\n") + "\n]\n")
}
