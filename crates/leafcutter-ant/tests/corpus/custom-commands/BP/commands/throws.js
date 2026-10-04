export default ({ name, template }) => {
	name('throws')
	template(() => {
		throw new Error('template failed')
	})
}
