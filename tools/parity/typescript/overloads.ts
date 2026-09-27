function f(a: string): string
function f(a: number): number
function f(a: any) { return a }
abstract class Base<T extends object = {}> { abstract run(): void; protected constructor(public readonly t: T) {} }
export default f
export abstract class X extends Base {}
