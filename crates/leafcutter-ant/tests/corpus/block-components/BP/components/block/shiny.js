export default defineComponent(({ name, template }) => {
	name('demo:shiny')
	template(({ light = 15 }, { create, lootTable, recipe, location, identifier, sourceBlock, projectNamespace }) => {
		create({ 'minecraft:light_emission': light }, location)
		create({ 'minecraft:loot': lootTable({ pools: [{ rolls: 1 }] }) }, location)
		recipe({ 'minecraft:recipe_shaped': { description: { identifier } } })
		create({ ns: projectNamespace, n: Object.keys(sourceBlock()['minecraft:block']).length }, 'minecraft:block/description/debug')
	})
})
