class K { static #n = 1; static { K.#n++ } m(this: K, a?: number, ...r: string[]) { return a ?? r.length } }
const o = { a: { b: 1 } }
console.log(o?.a?.b, K)
await Promise.resolve()
label: { break label }
