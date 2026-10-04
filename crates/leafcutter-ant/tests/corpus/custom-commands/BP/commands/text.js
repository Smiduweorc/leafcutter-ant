export default function ({ name, template }) {
	name('text')
	template((args) => `say ${args.join('|')}\n/say second line\n# comment line`)
}
