// A comment
import { world, system } from '@minecraft/server'
import type { Entity } from '@minecraft/server'
enum Color { Red, Green = 'g', Blue = 4 }
interface Foo { a: number }
/** doc */
export class Thing<T> implements Foo {
	a = 1
	static count: number = 0
	#priv = 2
	constructor(private readonly x: T, public y?: string) { }
	get value(): T { return this.x as T }
	async run(): Promise<void> { await Promise.resolve(); this.a ??= 3; const o = { ...{ b: 1 } }; o?.b; }
}
namespace NS { export const v = 1 }
export default function (e: Entity): number { return 1_000 + 0x10 + (e as any).id! }
const big = 10n ** 3n
label: for (const x of [1, 2]) { if (x) continue label }
try { JSON.parse('') } catch { }
declare const g: string
export const s = `tpl ${g}` + "dq" + 'sq'
