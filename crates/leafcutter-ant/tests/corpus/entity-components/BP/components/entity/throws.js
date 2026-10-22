export default defineComponent(({ name, template }) => {
	name('demo:throws')
	template(() => {
		throw new Error('template failed')
	})
})
