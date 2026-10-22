Bridge.register(class {
	static component_name = 'demo:legacy'
	onApply(data) {
		return { 'minecraft:item': { components: { 'minecraft:legacy': data } } }
	}
})
