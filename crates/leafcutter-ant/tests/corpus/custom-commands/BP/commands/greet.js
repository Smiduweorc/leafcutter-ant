export default defineCommand(({ name, schema, template }) => {
	name('greet')
	schema({ arguments: [] })
	template(([who = 'world', times = 1], { compilerMode, commandNestingDepth }) => {
		const lines = []
		for (let i = 0; i < times; i++) lines.push(`say Hello ${who} (${compilerMode}, depth ${commandNestingDepth})`)
		return lines
	})
})
