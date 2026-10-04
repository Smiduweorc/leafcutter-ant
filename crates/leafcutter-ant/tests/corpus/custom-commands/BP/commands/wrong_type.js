export default ({ name, template }) => {
	name('wrong_type')
	template(() => 42)
}
