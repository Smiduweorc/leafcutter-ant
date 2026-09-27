import type { A } from './a'
import { b, type C } from './b'
import * as ns from 'ns'
import def from 'def'
export { b }
export type { A }
export * from './c'
const c: C = b as C
console.log(ns, def, c)
