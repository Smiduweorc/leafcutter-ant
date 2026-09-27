const conf = { a: 1 } satisfies Record<string, number>
const enum E { X = 1, Y }
console.log(conf, E.Y)
let v = <number>(<unknown>'1')
let w = v!
