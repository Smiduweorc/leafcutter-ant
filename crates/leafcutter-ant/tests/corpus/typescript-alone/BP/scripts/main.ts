import { world } from '@minecraft/server'
enum Mode { A, B }
export const m: Mode = Mode.B
world.sendMessage(`${m}`)
