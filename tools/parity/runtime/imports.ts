import def from './def'
import { a, b as c } from './named'
import * as ns from 'ns'
import './side-effect.js'
import d2, { e } from '@bridge/generate'
import d3, * as ns2 from 'both'
import {} from 'nothing'
import type { T } from './types'
import { type U, f } from './mixed'
const x: T = def as T
console.log(x, a, c, ns, d2, e, d3, ns2, f)
