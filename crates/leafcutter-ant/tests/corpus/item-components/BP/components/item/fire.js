export default defineComponent(({ name, template }) => {
	name('demo:fire')
	template(({ seconds = 2 }, { create, player, recipe, lootTable, identifier, location, sourceItem }) => {
		create({ 'minecraft:on_use': { on_use: { event: 'demo:fire' } } }, 'minecraft:item/components')
		create({ 'demo:fire': { sequence: [{ run_command: { command: [`effect @s fire_resistance ${seconds}`] } }] } }, 'minecraft:item/events')
		player.animation({ timeline: { '0.0': [`/say ${identifier}`] } }, `q.is_item_name_any('slot.weapon.mainhand', 0, '${identifier}')`)
		player.animationController({ states: { default: {} } })
		player.create({ 'minecraft:type_family': { family: ['fire_user'] } }, 'minecraft:entity/components')
		player.create({ fire: { 'minecraft:scale': { value: 1 } } }, 'minecraft:entity/component_groups')
		recipe({ format_version: '1.12.0', 'minecraft:recipe_shapeless': { description: { identifier: `${identifier}_recipe` } } })
		lootTable({ pools: [] })
		create({ where: location, keys: Object.keys(sourceItem()) }, 'minecraft:item/description/debug')
	})
})
