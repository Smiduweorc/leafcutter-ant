import { world, system, Player } from '@minecraft/server'
system.runInterval(() => {
	for (const player of world.getPlayers() as Player[]) {
		player.sendMessage(`Hello ${player.name}`)
	}
}, 20)
