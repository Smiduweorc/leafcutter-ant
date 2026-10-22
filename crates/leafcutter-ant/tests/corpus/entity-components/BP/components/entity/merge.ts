export default defineComponent(({ name, template }) => {
	name('demo:merge')
	template((args: any, { create, dialogueScene }) => {
		create({ 'minecraft:type_family': { family: ['merged'] } }, 'minecraft:entity/components', (deepMerge: any, oldData: any, newData: any) => ({ ...oldData, replaced: newData }))
		create({ list: [args.a] }, 'minecraft:entity/description/extra')
		create({ list: [args.b] }, 'minecraft:entity/description/extra')
		dialogueScene({ scene_tag: 'hello', npc_name: 'Hi', text: 'Hello' })
		dialogueScene({ scene_tag: 'quiet', text: 'Shh' }, false)
		dialogueScene({ text: 'no tag' })
	})
})
