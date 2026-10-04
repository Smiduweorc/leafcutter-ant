import { createCollection, useTemplate } from '@bridge/generate'

const collection = createCollection()
const kept = await useTemplate('../templates/kept.json', { omitTemplate: false })
const text = await useTemplate('../templates/text.txt')
for (const color of ['red', 'green', 'blue']) {
	collection.add(`${color}.json`, {
		format_version: '1.16.100',
		'minecraft:item': { description: { identifier: `custom:${color}_gem` }, components: { kept } },
	})
}
collection.add('red.json', { overwritten: true })
collection.add('nested/notes.txt', text.toUpperCase())
collection.add('../loot_tables/from_items.json', { pools: [] })
export default collection
