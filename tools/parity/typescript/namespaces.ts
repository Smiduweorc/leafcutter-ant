namespace A.B { export const c = 1; export namespace D { export let e = c } }
declare module 'm' { export const z: number }
enum Dir { Up = 'UP', Down = 'DOWN' }
export { A, Dir }
